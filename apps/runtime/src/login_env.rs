//! What a host run as a person reads from their login shell (decision 151
//! N7). launchd and Task Scheduler start a job with a thin environment, so a
//! host started by a LaunchAgent would not find the CLIs the person's
//! terminal finds (Homebrew's, a version manager's). Once at start, a host
//! that is not the service ([`crate::platform::is_service_context`]) runs
//! `$SHELL -l -c` to read the login `PATH`, and any variables `import_env`
//! names, as the person's terminal would see them. It sets no policy: a
//! failure, a timeout or a missing shell imports nothing and is logged.

use std::ffi::OsString;
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// How long a login shell may take to print its environment.
pub const TIMEOUT: Duration = Duration::from_secs(5);

/// The first field the shell prints, so whatever a profile itself prints
/// on stdout is told apart from the values.
const MARKER: &str = "STORM_RUNTIME_LOGIN_ENV";

/// What the login shell had: its `PATH`, and the `import_env` variables it
/// set to a non-empty value, in the order the config names them.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LoginEnv {
    pub path: Option<OsString>,
    pub vars: Vec<(OsString, OsString)>,
}

/// Whether `name` can be an `import_env` entry: a shell variable name, and
/// not `PATH`, which is always read.
pub fn importable(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        && name != "PATH"
}

/// The `-c` script: the marker, `PATH`, then each importable name, each
/// followed by a NUL. Only [`importable`] names reach the shell.
pub fn script(names: &[String]) -> String {
    let mut script = format!("printf '%s\\0' {MARKER} \"$PATH\"");
    for name in names.iter().filter(|n| importable(n)) {
        script.push_str(&format!(" \"${name}\""));
    }
    script
}

/// The shell's output for [`script`] `names`. An empty value counts as
/// unset; output without the marker is nothing.
pub fn parse(out: &[u8], names: &[String]) -> LoginEnv {
    let mut fields = out.split(|b| *b == 0);
    if !fields
        .next()
        .is_some_and(|f| f.ends_with(MARKER.as_bytes()))
    {
        return LoginEnv::default();
    }
    let text = |f: &[u8]| String::from_utf8_lossy(f).into_owned();
    let path = fields
        .next()
        .filter(|f| !f.is_empty())
        .map(|f| OsString::from(text(f)));
    let vars = names
        .iter()
        .filter(|n| importable(n))
        .zip(fields)
        .filter(|(_, v)| !v.is_empty())
        .map(|(n, v)| (OsString::from(n), OsString::from(text(v))))
        .collect();
    LoginEnv { path, vars }
}

/// Runs `shell -l -c` [`script`] `names`, killed (with whatever it started)
/// after `timeout`.
pub fn capture(shell: &Path, names: &[String], timeout: Duration) -> Result<LoginEnv, String> {
    let mut command = Command::new(shell);
    command
        .args(["-l", "-c", &script(names)])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut command, 0);
    let mut child = command.spawn().map_err(|e| e.to_string())?;
    let mut stdout = child.stdout.take().ok_or("no stdout")?;
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut out = Vec::new();
        let _ = (&mut stdout).take(1 << 20).read_to_end(&mut out);
        let _ = tx.send(out);
    });
    let deadline = Instant::now() + timeout;
    let out = rx.recv_timeout(timeout);
    while out.is_ok() && Instant::now() < deadline {
        if !matches!(child.try_wait(), Ok(None)) {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    end_group(&child);
    let _ = child.kill();
    let _ = child.wait();
    let out = out.map_err(|_| format!("no answer within {timeout:?}"))?;
    let env = parse(&out, names);
    if env.path.is_none() {
        return Err("it printed no PATH".into());
    }
    Ok(env)
}

#[cfg(unix)]
fn end_group(child: &std::process::Child) {
    if let Some(pid) = rustix::process::Pid::from_raw(child.id() as i32) {
        let _ = rustix::process::kill_process_group(pid, rustix::process::Signal::KILL);
    }
}

#[cfg(not(unix))]
fn end_group(_: &std::process::Child) {}

/// The login environment of the person this host runs as, or nothing.
/// Windows: nothing; Task Scheduler's interactive token already gives the
/// person's environment, and there is no login shell. Under systemd (a
/// `systemd --user` unit), nothing; the environment is the user manager's.
pub fn for_host(names: &[String]) -> LoginEnv {
    if cfg!(windows) {
        if !names.is_empty() {
            tracing::info!(
                "import_env: Windows has no login shell; the host's environment is the account's own"
            );
        }
        return LoginEnv::default();
    }
    if std::env::var_os("INVOCATION_ID").is_some() {
        tracing::info!("login shell: under systemd, its environment is kept");
        return LoginEnv::default();
    }
    let Some(shell) = std::env::var_os("SHELL").filter(|s| !s.is_empty()) else {
        tracing::warn!("login shell: SHELL is not set; nothing imported");
        return LoginEnv::default();
    };
    match capture(Path::new(&shell), names, TIMEOUT) {
        Ok(env) => {
            let vars: Vec<_> = env.vars.iter().map(|(k, _)| k.to_string_lossy()).collect();
            tracing::info!(
                shell = %Path::new(&shell).display(),
                path = %env.path.as_deref().unwrap_or_default().to_string_lossy(),
                vars = ?vars,
                "login shell: imported its PATH"
            );
            env
        }
        Err(e) => {
            tracing::warn!(shell = %Path::new(&shell).display(), error = %e, "login shell: nothing imported");
            LoginEnv::default()
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn names(n: &[&str]) -> Vec<String> {
        n.iter().map(|s| s.to_string()).collect()
    }

    fn fake_shell(dir: &Path, body: &str) -> std::path::PathBuf {
        let shell = dir.join("fake-shell");
        std::fs::write(&shell, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&shell, std::fs::Permissions::from_mode(0o755)).unwrap();
        shell
    }

    #[test]
    fn only_the_named_shell_variables_are_imported() {
        assert!(importable("HOMEBREW_PREFIX"));
        assert!(importable("_x1"));
        for bad in ["", "PATH", "1X", "A-B", "A B", "$(rm)", "A\"B"] {
            assert!(!importable(bad), "{bad:?}");
        }
        let asked = names(&["A", "$(touch x)", "B", "C"]);
        let s = script(&asked);
        assert_eq!(
            s,
            format!("printf '%s\\0' {MARKER} \"$PATH\" \"$A\" \"$B\" \"$C\"")
        );
        let out = format!("profile noise\n{MARKER}\0/opt/bin:/bin\0one\0\0three\0");
        assert_eq!(
            parse(out.as_bytes(), &asked),
            LoginEnv {
                path: Some("/opt/bin:/bin".into()),
                vars: vec![("A".into(), "one".into()), ("C".into(), "three".into())],
            }
        );
        assert_eq!(parse(b"/bin\0x\0", &asked), LoginEnv::default());
    }

    #[test]
    fn a_fake_login_shell_gives_its_path_and_named_variables() {
        let dir = tempfile::tempdir().unwrap();
        let shell = fake_shell(
            dir.path(),
            "[ \"$1\" = -l ] || exit 9\necho from the profile\n\
             export PATH=/opt/homebrew/bin:/usr/bin:/bin WANTED=yes OTHER=no\n\
             shift\nexec /bin/sh \"$@\"",
        );
        let env = capture(&shell, &names(&["WANTED", "MISSING"]), TIMEOUT).unwrap();
        assert_eq!(
            env.path.as_deref(),
            Some("/opt/homebrew/bin:/usr/bin:/bin".as_ref())
        );
        assert_eq!(env.vars, vec![("WANTED".into(), "yes".into())]);
    }

    #[test]
    fn a_login_shell_that_hangs_is_killed_at_the_timeout() {
        let dir = tempfile::tempdir().unwrap();
        let shell = fake_shell(dir.path(), "sleep 30 &\nsleep 30");
        let started = Instant::now();
        let e = capture(&shell, &[], Duration::from_millis(300)).unwrap_err();
        assert!(e.contains("no answer"), "{e}");
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn a_missing_shell_imports_nothing() {
        let dir = tempfile::tempdir().unwrap();
        assert!(capture(&dir.path().join("no-such-shell"), &[], TIMEOUT).is_err());
        let silent = fake_shell(dir.path(), "exit 0");
        assert!(capture(&silent, &[], TIMEOUT).is_err());
    }
}
