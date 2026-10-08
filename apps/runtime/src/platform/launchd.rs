//! The macOS service definition (D14, AM33–AM35): the layout under
//! `/Library/StormRuntime`, the LaunchDaemon plist and the config `install`
//! writes.
//!
//! Pure functions, compiled on every platform, so the Linux suite tests the
//! plist too. Executing them (accounts, modes, `launchctl`) is macOS's
//! `service` module.

use std::path::{Path, PathBuf};

/// The host's state directory on macOS: `platform::DEFAULT_STATE` there.
pub const STATE: &str = "/Library/StormRuntime/state";

/// The operator's `runtime.toml` on macOS: `platform::DEFAULT_CONFIG` there.
pub const CONFIG: &str = "/Library/StormRuntime/runtime.toml";

/// The `PATH` a macOS host and its agents get when `runtime.toml` names none
/// (AM38). `$HOME` is the account's home, where a CLI the account installed
/// for itself lands; then Homebrew on Apple silicon and Intel, then the
/// system.
pub const DEFAULT_PATH: &str = "$HOME/.local/bin:/opt/homebrew/bin:/opt/homebrew/sbin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin";

/// [`DEFAULT_PATH`] for an account whose home is `home`.
pub fn path_for_home(home: &str) -> String {
    DEFAULT_PATH.replace("$HOME", home)
}

/// The job's label, under the app's bundle-id prefix (`dev.storm`).
pub const LABEL: &str = "dev.storm.runtime";

/// The account's login shell: nobody logs in as it (AM34).
pub const NO_LOGIN_SHELL: &str = "/usr/bin/false";

/// The shell a `shell` session starts (AM38). launchd would otherwise pass
/// the account's own shell, which is [`NO_LOGIN_SHELL`].
pub const SESSION_SHELL: &str = "/bin/zsh";

/// Where everything a macOS host owns lives (AM33). No path contains a
/// space: workspace paths reach agents' build tools.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    pub root: PathBuf,
    pub bin: PathBuf,
    pub config: PathBuf,
    pub state: PathBuf,
    pub home: PathBuf,
    pub workspaces: PathBuf,
    pub logs: PathBuf,
    pub log_file: PathBuf,
    pub plist: PathBuf,
}

impl Layout {
    pub fn standard() -> Self {
        let root = PathBuf::from("/Library/StormRuntime");
        let state = PathBuf::from(STATE);
        let logs = PathBuf::from("/Library/Logs/StormRuntime");
        Self {
            bin: root.join("bin/storm-runtime"),
            config: PathBuf::from(CONFIG),
            home: state.join("home"),
            workspaces: root.join("workspaces"),
            log_file: logs.join("storm-runtime.log"),
            plist: PathBuf::from(format!("/Library/LaunchDaemons/{LABEL}.plist")),
            root,
            state,
            logs,
        }
    }

    /// The file whose existence means "enrolled", and the job's `PathState`.
    pub fn enrolled_marker(&self) -> PathBuf {
        self.state.join(crate::identity::HOST_FILE)
    }

    /// The `PATH` the daemon and its agents get when `runtime.toml` names
    /// none (AM38).
    pub fn default_path(&self) -> String {
        path_for_home(&self.home.display().to_string())
    }
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn string(s: impl AsRef<str>) -> String {
    format!("<string>{}</string>", escape(s.as_ref()))
}

fn path(p: &Path) -> String {
    string(p.display().to_string())
}

/// The LaunchDaemon (AM35).
///
/// - It runs as the dedicated account, with an explicit environment: launchd
///   would otherwise hand agents a bare `PATH` and the account's
///   `/usr/bin/false` as `SHELL`.
/// - `KeepAlive` is `PathState` on `host.json`: launchd runs the host exactly
///   while it is enrolled. Enrolling starts it, a crash restarts it, and a
///   revoked host (AM36 renames `host.json`) stays stopped — launchd has no
///   `RestartPreventExitStatus`.
/// - `ProcessType Standard`, never `Background`, which would throttle CPU and
///   I/O and keep agents off the performance cores.
pub fn plist(layout: &Layout, account: &str) -> String {
    let args = [
        path(&layout.bin),
        string("serve"),
        string("--state"),
        path(&layout.state),
        string("--config"),
        path(&layout.config),
    ];
    let env = [
        ("HOME", layout.home.display().to_string()),
        ("PATH", layout.default_path()),
        ("SHELL", SESSION_SHELL.to_string()),
        ("LANG", "en_US.UTF-8".to_string()),
    ];
    let mut out = String::new();
    out.push_str(concat!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
        "<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" ",
        "\"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n",
        "<!-- Written by `storm-runtime install` (PLAN.md decision 83, D14 AM35).\n",
        "     Re-run `sudo storm-runtime install` rather than editing this file. -->\n",
        "<plist version=\"1.0\">\n<dict>\n",
    ));
    out.push_str(&format!("  <key>Label</key>\n  {}\n", string(LABEL)));
    out.push_str("  <key>ProgramArguments</key>\n  <array>\n");
    for a in &args {
        out.push_str(&format!("    {a}\n"));
    }
    out.push_str("  </array>\n");
    out.push_str(&format!("  <key>UserName</key>\n  {}\n", string(account)));
    out.push_str(&format!("  <key>GroupName</key>\n  {}\n", string(account)));
    out.push_str("  <key>EnvironmentVariables</key>\n  <dict>\n");
    for (k, v) in &env {
        out.push_str(&format!("    <key>{k}</key>\n    {}\n", string(v)));
    }
    out.push_str("  </dict>\n");
    out.push_str(&format!(
        "  <key>WorkingDirectory</key>\n  {}\n",
        path(&layout.state)
    ));
    out.push_str(&format!(
        concat!(
            "  <key>KeepAlive</key>\n  <dict>\n",
            "    <key>PathState</key>\n    <dict>\n",
            "      <key>{}</key>\n      <true/>\n",
            "    </dict>\n  </dict>\n",
        ),
        escape(&layout.enrolled_marker().display().to_string())
    ));
    out.push_str("  <key>ThrottleInterval</key>\n  <integer>10</integer>\n");
    out.push_str("  <key>ExitTimeOut</key>\n  <integer>20</integer>\n");
    out.push_str("  <key>ProcessType</key>\n  <string>Standard</string>\n");
    // 0o027, which plists spell in decimal.
    out.push_str("  <key>Umask</key>\n  <integer>23</integer>\n");
    out.push_str(&format!(
        "  <key>StandardOutPath</key>\n  {}\n",
        path(&layout.log_file)
    ));
    out.push_str(&format!(
        "  <key>StandardErrorPath</key>\n  {}\n",
        path(&layout.log_file)
    ));
    out.push_str("</dict>\n</plist>\n");
    out
}

/// The `safe.directory` value that lets git work in every workspace,
/// whichever account created the checkout. Git refuses a repository owned
/// by another uid ("dubious ownership") and ignores ACLs, so without it the
/// agent cannot use a checkout the operator made, nor the operator one the
/// agent made (AM39's shared root).
pub fn git_safe_directory(layout: &Layout) -> String {
    format!("{}/*", layout.workspaces.display())
}

/// `gitconfig` with [`git_safe_directory`] added, or `None` when it is
/// already there. Appended as its own `[safe]` section, so the rest of the
/// file is left exactly as it was.
pub fn with_git_safe_directory(gitconfig: &str, layout: &Layout) -> Option<String> {
    let value = git_safe_directory(layout);
    let present = gitconfig.lines().any(|l| {
        l.split_once('=')
            .is_some_and(|(k, v)| k.trim() == "directory" && v.trim() == value)
    });
    if present {
        return None;
    }
    let mut out = gitconfig.to_string();
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(&format!("[safe]\n\tdirectory = {value}\n"));
    Some(out)
}

/// The `runtime.toml` `install` writes when there is none (AM33, AM39). The
/// operator's afterwards: `install` never overwrites it.
pub fn default_config(layout: &Layout) -> String {
    format!(
        r#"# {config}: the Storm Runtime Host's configuration
# (freeze §5.7; macOS, D14). Every value here is an operational default.
# Written once by `storm-runtime install`, which never overwrites it.

# Each subdirectory of a root is a workspace, named by its directory name.
# The default root is Storm's own, outside every user home, so the host needs
# no Full Disk Access (AM39). You and the host's account can both read and
# write it.
workspace_roots = ["{workspaces}"]

max_sessions = 8
scrollback_bytes = 4194304

# Where agent CLIs are looked for, and the PATH every session gets (AM38).
# Omit it for the default:
#   {path}
# A CLI must be executable by the host's account: a CLI inside a person's
# home is out of its reach.
# path = ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/bin"]

# Omit [[providers]] entirely to offer the three built-ins: claude-code (the
# default), opencode and shell. Provider secrets go in an env file, mode 0600,
# readable by the host's account, never here:
#
# [[providers]]
# id = "claude-code"
# env_file = "{state}/claude.env"
"#,
        config = layout.config.display(),
        workspaces = layout.workspaces.display(),
        state = layout.state.display(),
        path = layout.default_path(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn value_after<'a>(plist: &'a str, key: &str) -> &'a str {
        let at = plist
            .find(&format!("<key>{key}</key>"))
            .unwrap_or_else(|| panic!("no {key}"));
        plist[at..].lines().nth(1).unwrap().trim()
    }

    #[test]
    fn git_trusts_the_workspace_root_once_and_leaves_the_rest_alone() {
        let l = Layout::standard();
        let fresh = with_git_safe_directory("", &l).unwrap();
        assert_eq!(
            fresh,
            "[safe]\n\tdirectory = /Library/StormRuntime/workspaces/*\n"
        );
        assert_eq!(with_git_safe_directory(&fresh, &l), None, "added twice");
        let mine = "[user]\n\tname = Agent";
        let merged = with_git_safe_directory(mine, &l).unwrap();
        assert!(
            merged.starts_with("[user]\n\tname = Agent\n[safe]\n"),
            "{merged}"
        );
        // A different directory is not this one.
        assert!(with_git_safe_directory("[safe]\n\tdirectory = /elsewhere/*\n", &l).is_some());
    }

    #[test]
    fn the_layout_is_amendment_33s_and_has_no_spaces() {
        let l = Layout::standard();
        assert_eq!(l.bin, Path::new("/Library/StormRuntime/bin/storm-runtime"));
        assert_eq!(l.config, Path::new("/Library/StormRuntime/runtime.toml"));
        assert!(l.root.join("state") == l.state && l.state.join("home") == l.home);
        assert_eq!(l.home, Path::new("/Library/StormRuntime/state/home"));
        assert_eq!(l.workspaces, Path::new("/Library/StormRuntime/workspaces"));
        assert_eq!(
            l.plist,
            Path::new("/Library/LaunchDaemons/dev.storm.runtime.plist")
        );
        for p in [
            &l.bin,
            &l.config,
            &l.state,
            &l.home,
            &l.workspaces,
            &l.log_file,
        ] {
            assert!(!p.display().to_string().contains(' '), "{}", p.display());
        }
    }

    #[test]
    fn the_daemon_runs_as_its_account_with_an_explicit_environment() {
        let l = Layout::standard();
        let p = plist(&l, "_stormruntime");
        assert_eq!(
            value_after(&p, "Label"),
            "<string>dev.storm.runtime</string>"
        );
        assert_eq!(
            value_after(&p, "UserName"),
            "<string>_stormruntime</string>"
        );
        assert_eq!(
            value_after(&p, "GroupName"),
            "<string>_stormruntime</string>"
        );
        assert_eq!(
            value_after(&p, "HOME"),
            "<string>/Library/StormRuntime/state/home</string>"
        );
        assert_eq!(value_after(&p, "SHELL"), "<string>/bin/zsh</string>");
        assert_eq!(value_after(&p, "LANG"), "<string>en_US.UTF-8</string>");
        let path = value_after(&p, "PATH");
        assert!(
            path.starts_with(
                "<string>/Library/StormRuntime/state/home/.local/bin:/opt/homebrew/bin:"
            ),
            "{path}"
        );
        assert!(!path.contains("$HOME"), "{path}");
        assert!(p.contains(
            "<string>/Library/StormRuntime/bin/storm-runtime</string>\n    <string>serve</string>"
        ));
    }

    #[test]
    fn launchd_runs_the_host_exactly_while_it_is_enrolled() {
        // AM35 + AM36: no RunAtLoad, and KeepAlive is PathState on host.json,
        // so an unenrolled or revoked host is never started or restarted.
        let p = plist(&Layout::standard(), "_stormruntime");
        assert!(
            !p.contains("RunAtLoad"),
            "an unenrolled host must not start"
        );
        let keep = &p[p.find("<key>KeepAlive</key>").unwrap()..];
        let keep = &keep[..keep.find("</dict>\n  </dict>").unwrap()];
        assert!(keep.contains("<key>PathState</key>"), "{keep}");
        assert!(
            keep.contains("<key>/Library/StormRuntime/state/host.json</key>\n      <true/>"),
            "{keep}"
        );
        assert!(!keep.contains("SuccessfulExit"), "{keep}");
    }

    #[test]
    fn the_macos_default_path_starts_at_the_accounts_own_clis_then_homebrew() {
        assert_eq!(
            path_for_home("/h"),
            "/h/.local/bin:/opt/homebrew/bin:/opt/homebrew/sbin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin"
        );
        // The plist's PATH and the host's default are the same string.
        let l = Layout::standard();
        assert_eq!(
            l.default_path(),
            path_for_home(&l.home.display().to_string())
        );
    }

    #[test]
    fn the_daemon_is_never_throttled_as_background_work() {
        let p = plist(&Layout::standard(), "_stormruntime");
        assert_eq!(value_after(&p, "ProcessType"), "<string>Standard</string>");
        assert!(!p.contains("Background"));
        assert_eq!(value_after(&p, "Umask"), "<integer>23</integer>");
        assert_eq!(23, 0o027, "plists spell the umask in decimal");
        // Longer than a session's SIGHUP→SIGKILL grace (AM37).
        assert_eq!(value_after(&p, "ExitTimeOut"), "<integer>20</integer>");
    }

    #[test]
    fn the_plist_is_well_formed() {
        let p = plist(&Layout::standard(), "a&b<c>");
        assert!(p.contains("<string>a&amp;b&lt;c&gt;</string>"));
        let opens = p.matches("<dict>").count();
        assert_eq!(opens, p.matches("</dict>").count());
        assert_eq!(p.matches("<array>").count(), p.matches("</array>").count());
        assert!(p.trim_end().ends_with("</plist>"));
    }

    #[test]
    fn the_default_config_parses_and_names_the_storm_workspace_root() {
        let l = Layout::standard();
        let text = default_config(&l);
        let config: crate::config::RuntimeConfig = toml::from_str(&text).expect("parses");
        assert_eq!(config.workspace_roots, vec![l.workspaces.clone()]);
        assert!(config.path.is_none(), "the default PATH is the platform's");
    }
}
