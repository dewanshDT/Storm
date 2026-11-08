//! The macOS service definition (D14, AM33–AM35): the layout under
//! `/Library/StormRuntime`, the LaunchDaemon plist and the config `install`
//! writes; and the LaunchAgent `install --user` writes for a host a person
//! runs as themselves (decision 151 E2).
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

/// macOS's `platform::PROTECTED_ROOTS` (AM39). `/System/Volumes/Data` is
/// where `/Users` and `/Volumes` really live (firmlinks): a root spelled
/// through it would otherwise slip past the first two.
pub const PROTECTED_ROOTS: &[&str] = &["/Users", "/Volumes", "/Network", "/System/Volumes/Data"];

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
        // The account runs nothing but this host: at start and after
        // shutdown, every other process of it is ended (AM37). launchd kills
        // only the job's own process group, and sessions have their own.
        string("--exclusive-account"),
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
        "<!-- Written by `storm-runtime install` (decision 83, D14 AM35).\n",
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

/// Homebrew's label for `brew services start storm-runtime`, which runs the
/// same host as [`user_plist`] from the same state.
pub const BREW_LABEL: &str = "homebrew.mxcl.storm-runtime";

/// Where `install --user` puts a host a person runs as themselves, a
/// LaunchAgent (decision 151 E2). `state` and `config` are N2's per-user
/// paths, so `enroll` and `check` with no flags find the same host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserLayout {
    pub home: PathBuf,
    pub root: PathBuf,
    pub bin: PathBuf,
    pub config: PathBuf,
    pub state: PathBuf,
    pub logs: PathBuf,
    pub log_file: PathBuf,
    pub agents: PathBuf,
    pub plist: PathBuf,
    pub brew_plist: PathBuf,
}

impl UserLayout {
    pub fn for_home(home: &Path) -> Self {
        let root = home.join("Library/Application Support/StormRuntime");
        let logs = home.join("Library/Logs/StormRuntime");
        let agents = home.join("Library/LaunchAgents");
        Self {
            home: home.to_path_buf(),
            bin: root.join("bin/storm-runtime"),
            config: root.join("runtime.toml"),
            state: root.join("state"),
            log_file: logs.join("storm-runtime.log"),
            plist: agents.join(format!("{LABEL}.plist")),
            brew_plist: agents.join(format!("{BREW_LABEL}.plist")),
            root,
            logs,
            agents,
        }
    }

    pub fn enrolled_marker(&self) -> PathBuf {
        self.state.join(crate::identity::HOST_FILE)
    }
}

/// The LaunchAgent (decision 151 E2): the host as the person who loaded it,
/// in their own session, so their login Keychain and their privacy prompts
/// are theirs. No `UserName`, no account sweep, and no environment beyond
/// `LANG`: launchd gives an agent the person's `HOME`, `USER` and `SHELL`,
/// and the host reads `PATH` from their login shell (N7).
///
/// `KeepAlive` is `PathState` on `host.json`, as for the LaunchDaemon: an
/// unenrolled or revoked host is never started, enrolling starts it, and a
/// crash restarts it. Logging out ends it; the next login starts it again.
pub fn user_plist(layout: &UserLayout) -> String {
    let args = [
        path(&layout.bin),
        string("serve"),
        string("--state"),
        path(&layout.state),
        string("--config"),
        path(&layout.config),
    ];
    let mut out = String::new();
    out.push_str(concat!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
        "<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" ",
        "\"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n",
        "<!-- Written by `storm-runtime install --user` (decision 151 E2).\n",
        "     Re-run it rather than editing this file. -->\n",
        "<plist version=\"1.0\">\n<dict>\n",
    ));
    out.push_str(&format!("  <key>Label</key>\n  {}\n", string(LABEL)));
    out.push_str("  <key>ProgramArguments</key>\n  <array>\n");
    for a in &args {
        out.push_str(&format!("    {a}\n"));
    }
    out.push_str("  </array>\n");
    out.push_str("  <key>EnvironmentVariables</key>\n  <dict>\n");
    out.push_str(&format!(
        "    <key>LANG</key>\n    {}\n",
        string("en_US.UTF-8")
    ));
    out.push_str("  </dict>\n");
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

/// The `runtime.toml` `install --user` writes when there is none. It names
/// no workspace root: which of a person's folders an agent may work in is
/// theirs to say.
pub fn user_default_config(layout: &UserLayout) -> String {
    format!(
        r#"# {config}: the configuration of the Storm Runtime Host you run as
# yourself (decision 151 E2). Every value here is an operational default.
# Written once by `storm-runtime install --user`, which never overwrites it.
# After editing it: launchctl kickstart -k gui/$(id -u)/{label}

# Each subdirectory of a root is a workspace, named by its directory name.
# There is none until you name one. A root may be in your home; one under
# ~/Documents, ~/Desktop or ~/Downloads makes macOS ask you first.
# workspace_roots = ["{home}/Code"]

max_sessions = 8
scrollback_bytes = 4194304

# The host finds agent CLIs on your login shell's PATH. Set one here to
# choose it yourself:
# path = ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/bin"]
"#,
        config = layout.config.display(),
        home = layout.home.display(),
        label = LABEL,
    )
}

/// What `install --user` found before acting.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UserFacts {
    pub uid: u32,
    /// Running as root, through `sudo`.
    pub root: bool,
    /// Running as the service's own account.
    pub service_account: bool,
    /// The person has a login session (`gui/<uid>` exists).
    pub gui_session: bool,
    pub config_exists: bool,
    pub brew_service: bool,
    pub daemon_installed: bool,
    /// A Storm server runs as this person (decision 84d's `server.json`).
    pub server_here: bool,
}

/// One thing `install --user` does, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UserStep {
    /// A directory only the person can enter.
    PrivateDir(PathBuf),
    /// `~/Library/LaunchAgents`, if it is missing.
    Dir(PathBuf),
    /// This binary, copied to `bin`.
    Binary,
    Config,
    Plist,
    /// Boot the job out of `domain` and back in.
    Load {
        domain: String,
    },
}

/// What `install --user` does on a Mac where it found `facts`, and what it
/// says first; or why it refuses.
pub fn plan_user_install(
    layout: &UserLayout,
    facts: &UserFacts,
) -> Result<(Vec<UserStep>, Vec<String>), String> {
    if facts.root {
        return Err(
            "install --user installs the host as you: run it without sudo \
                    (`sudo storm-runtime install` is the LaunchDaemon)"
                .into(),
        );
    }
    if facts.service_account {
        return Err(format!(
            "install --user runs a host as a person; this is the service's own account \
             {}: run `sudo storm-runtime install` instead",
            super::paths::MACOS_ACCOUNT
        ));
    }
    if facts.brew_service {
        return Err(format!(
            "`brew services` already runs this host from the same state ({}): use one. \
             `brew services stop storm-runtime` first, or keep brew's",
            layout.brew_plist.display()
        ));
    }
    let mut notes = Vec::new();
    if facts.daemon_installed {
        notes.push(format!(
            "The LaunchDaemon is installed too ({}): it is a separate host, as {}, with its \
             own state. `sudo /Library/StormRuntime/bin/storm-runtime uninstall` removes it.",
            Layout::standard().plist.display(),
            super::paths::MACOS_ACCOUNT
        ));
    }
    if facts.server_here {
        notes.push(
            "A Storm server runs as you on this Mac, so this host's agents can read its \
             vaults on disk. To keep them out, run the host as its own account instead: \
             `sudo storm-runtime install`."
                .into(),
        );
    }
    let mut steps = vec![
        UserStep::PrivateDir(layout.root.clone()),
        UserStep::PrivateDir(layout.bin.parent().expect("bin has a parent").to_path_buf()),
        UserStep::PrivateDir(layout.state.clone()),
        UserStep::PrivateDir(layout.logs.clone()),
        UserStep::Dir(layout.agents.clone()),
        UserStep::Binary,
    ];
    if !facts.config_exists {
        steps.push(UserStep::Config);
    }
    steps.push(UserStep::Plist);
    // An SSH-only login has no GUI domain: the agent lives in the
    // background one until the next login loads it into the GUI.
    let domain = if facts.gui_session { "gui" } else { "user" };
    steps.push(UserStep::Load {
        domain: format!("{domain}/{}", facts.uid),
    });
    Ok((steps, notes))
}

/// `s` as one word for a POSIX shell: the per-user paths have a space.
pub fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
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
    with_safe_directory(gitconfig, &git_safe_directory(layout))
}

/// `gitconfig` with `safe.directory = value` added, or `None` when it is
/// already there. Windows' install uses it too (AM54).
pub fn with_safe_directory(gitconfig: &str, value: &str) -> Option<String> {
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
# write it. Roots under /Users, /Volumes or /Network are refused: to work on
# an existing checkout, clone or move it into a workspace here.
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

// macOS paths, built with `Path::join`, so only where `/` is the separator.
#[cfg(all(test, unix))]
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
    fn the_daemon_sweeps_its_account_like_the_systemd_unit() {
        // AM37: launchd kills only the job's process group, so the sweep is
        // what leaves no survivor after a stop or a crash.
        let p = plist(&Layout::standard(), "_stormruntime");
        let args = &p[p.find("<key>ProgramArguments</key>").unwrap()..];
        let args = &args[..args.find("</array>").unwrap()];
        assert!(
            args.contains("<string>--exclusive-account</string>"),
            "{args}"
        );
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

    fn alice() -> UserLayout {
        UserLayout::for_home(Path::new("/Users/alice"))
    }

    fn args_of(plist: &str) -> Vec<&str> {
        let a = &plist[plist.find("<key>ProgramArguments</key>").unwrap()..];
        a[..a.find("</array>").unwrap()]
            .lines()
            .filter_map(|l| l.trim().strip_prefix("<string>")?.strip_suffix("</string>"))
            .collect()
    }

    #[test]
    fn the_user_layout_is_n2s_per_user_paths() {
        let l = alice();
        let n2 = super::super::paths::resolve(super::super::paths::Os::Macos, false, |n| {
            (n == "HOME").then(|| "/Users/alice".into())
        })
        .unwrap();
        assert_eq!(l.state, n2.state);
        assert_eq!(l.config, n2.config);
        assert_eq!(
            l.bin,
            Path::new("/Users/alice/Library/Application Support/StormRuntime/bin/storm-runtime")
        );
        assert_eq!(
            l.plist,
            Path::new("/Users/alice/Library/LaunchAgents/dev.storm.runtime.plist")
        );
        assert_eq!(
            l.brew_plist,
            Path::new("/Users/alice/Library/LaunchAgents/homebrew.mxcl.storm-runtime.plist")
        );
        assert_eq!(
            l.log_file,
            Path::new("/Users/alice/Library/Logs/StormRuntime/storm-runtime.log")
        );
    }

    #[test]
    fn the_launch_agent_runs_serve_on_the_per_user_paths_as_whoever_loads_it() {
        let p = user_plist(&alice());
        assert_eq!(
            value_after(&p, "Label"),
            "<string>dev.storm.runtime</string>"
        );
        // Each path is one argument, space and all: launchd runs no shell.
        assert_eq!(
            args_of(&p),
            [
                "/Users/alice/Library/Application Support/StormRuntime/bin/storm-runtime",
                "serve",
                "--state",
                "/Users/alice/Library/Application Support/StormRuntime/state",
                "--config",
                "/Users/alice/Library/Application Support/StormRuntime/runtime.toml",
            ]
        );
        for absent in [
            "UserName",
            "GroupName",
            "Umask",
            "--exclusive-account",
            "HOME",
            "PATH",
        ] {
            assert!(!p.contains(absent), "{absent} in {p}");
        }
        assert_eq!(value_after(&p, "LANG"), "<string>en_US.UTF-8</string>");
        assert_eq!(value_after(&p, "ProcessType"), "<string>Standard</string>");
        let log = "<string>/Users/alice/Library/Logs/StormRuntime/storm-runtime.log</string>";
        assert_eq!(value_after(&p, "StandardOutPath"), log);
        assert_eq!(value_after(&p, "StandardErrorPath"), log);
    }

    #[test]
    fn the_launch_agent_runs_exactly_while_the_host_is_enrolled() {
        let p = user_plist(&alice());
        assert!(
            !p.contains("RunAtLoad"),
            "an unenrolled host must not start"
        );
        let keep = &p[p.find("<key>KeepAlive</key>").unwrap()..];
        let keep = &keep[..keep.find("</dict>\n  </dict>").unwrap()];
        assert!(
            keep.contains(
                "<key>/Users/alice/Library/Application Support/StormRuntime/state/host.json</key>\n      <true/>"
            ),
            "{keep}"
        );
        assert_eq!(p.matches("<dict>").count(), p.matches("</dict>").count());
        assert!(p.trim_end().ends_with("</plist>"));
        let odd = user_plist(&UserLayout::for_home(Path::new("/Users/a&b")));
        assert!(odd.contains("<string>/Users/a&amp;b/Library/"), "{odd}");
    }

    #[test]
    fn the_user_config_parses_and_names_no_root() {
        let text = user_default_config(&alice());
        let config: crate::config::RuntimeConfig = toml::from_str(&text).expect("parses");
        assert!(config.workspace_roots.is_empty());
        assert!(config.path.is_none());
        assert!(
            text.contains("# workspace_roots = [\"/Users/alice/Code\"]"),
            "{text}"
        );
    }

    fn facts() -> UserFacts {
        UserFacts {
            uid: 501,
            gui_session: true,
            ..UserFacts::default()
        }
    }

    #[test]
    fn a_fresh_user_install_makes_private_dirs_then_loads_into_the_gui_domain() {
        let l = alice();
        let (steps, notes) = plan_user_install(&l, &facts()).unwrap();
        assert!(notes.is_empty(), "{notes:?}");
        assert_eq!(
            steps,
            [
                UserStep::PrivateDir(l.root.clone()),
                UserStep::PrivateDir(l.root.join("bin")),
                UserStep::PrivateDir(l.state.clone()),
                UserStep::PrivateDir(l.logs.clone()),
                UserStep::Dir(l.agents.clone()),
                UserStep::Binary,
                UserStep::Config,
                UserStep::Plist,
                UserStep::Load {
                    domain: "gui/501".into()
                },
            ]
        );
    }

    #[test]
    fn a_user_reinstall_keeps_the_config_and_without_a_login_uses_the_user_domain() {
        let f = UserFacts {
            config_exists: true,
            gui_session: false,
            ..facts()
        };
        let (steps, _) = plan_user_install(&alice(), &f).unwrap();
        assert!(!steps.contains(&UserStep::Config));
        assert_eq!(
            steps.last(),
            Some(&UserStep::Load {
                domain: "user/501".into()
            })
        );
    }

    #[test]
    fn a_user_install_refuses_sudo_the_service_account_and_brew_services() {
        let l = alice();
        let err = plan_user_install(
            &l,
            &UserFacts {
                root: true,
                ..facts()
            },
        )
        .unwrap_err();
        assert!(err.contains("without sudo"), "{err}");
        let err = plan_user_install(
            &l,
            &UserFacts {
                service_account: true,
                ..facts()
            },
        )
        .unwrap_err();
        assert!(err.contains("_stormruntime"), "{err}");
        let err = plan_user_install(
            &l,
            &UserFacts {
                brew_service: true,
                ..facts()
            },
        )
        .unwrap_err();
        assert!(err.contains("brew services stop storm-runtime"), "{err}");
    }

    #[test]
    fn a_user_install_beside_the_daemon_or_a_server_says_so() {
        let f = UserFacts {
            daemon_installed: true,
            server_here: true,
            ..facts()
        };
        let (steps, notes) = plan_user_install(&alice(), &f).unwrap();
        assert!(steps.contains(&UserStep::Plist));
        assert_eq!(notes.len(), 2, "{notes:?}");
        assert!(notes[0].contains("separate host"), "{notes:?}");
        assert!(notes[1].contains("can read its vaults"), "{notes:?}");
    }

    #[test]
    fn a_path_with_a_space_or_a_quote_is_one_shell_word() {
        assert_eq!(
            shell_quote("/Users/a/Library/Application Support/x"),
            "'/Users/a/Library/Application Support/x'"
        );
        assert_eq!(shell_quote("it's"), r"'it'\''s'");
    }
}
