//! A host you run as yourself on Linux, under your systemd user manager
//! (decision 151 E1): what `storm-runtime install --user` and `uninstall
//! --user` do, decided here and carried out by `platform::linux`.
//!
//! Pure functions of the environment and what is on disk, compiled on every
//! platform like `paths`, so the unit and the plans are tested anywhere.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use super::paths::{self, Os, unix_join};

pub const UNIT_NAME: &str = "storm-runtime.service";

/// The unit, with `ExecStart=@EXEC_START@` for [`render_unit`] to fill.
pub const TEMPLATE: &str = include_str!("../../../../deploy/storm-runtime-user.service");

const EXEC_START: &str = "ExecStart=@EXEC_START@";

/// The first line of a unit whose linger `install --user` turned on, so
/// `uninstall --user` turns it off again and leaves no file under
/// `/var/lib/systemd/linger`.
pub const LINGER_MARKER: &str =
    "# Linger: turned on by `storm-runtime install --user`; `uninstall --user` turns it off.";

/// Where a person's own host lives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    /// `~/.local/bin/storm-runtime`, the copy the unit runs.
    pub bin: PathBuf,
    /// `$XDG_CONFIG_HOME/systemd/user/storm-runtime.service`.
    pub unit: PathBuf,
    /// The link `systemctl --user enable` makes.
    pub wants: PathBuf,
    /// N2's state directory and `runtime.toml` for a person.
    pub state: PathBuf,
    pub config: PathBuf,
    /// The workspace root a fresh `runtime.toml` names.
    pub workspaces: PathBuf,
}

pub fn layout(env: impl Fn(&str) -> Option<OsString>) -> Result<Layout, String> {
    let host = paths::resolve(Os::Linux, false, &env)?;
    let var = |name: &str| env(name).filter(|v| !v.is_empty()).map(PathBuf::from);
    let home = var("HOME").ok_or("HOME is not set, so there is no home to install into")?;
    let config_home = var("XDG_CONFIG_HOME")
        .filter(|p| is_absolute(p))
        .unwrap_or_else(|| unix_join(&home, ".config"));
    let units = unix_join(&config_home, "systemd/user");
    Ok(Layout {
        bin: unix_join(&home, ".local/bin/storm-runtime"),
        unit: unix_join(&units, UNIT_NAME),
        wants: unix_join(&units, &format!("default.target.wants/{UNIT_NAME}")),
        workspaces: paths::user_workspaces(Os::Linux, &host.state),
        state: host.state,
        config: host.config,
    })
}

/// Absolute on Linux, wherever this is compiled: on Windows `/home` is not.
fn is_absolute(path: &Path) -> bool {
    path.to_string_lossy().starts_with('/')
}

/// One `ExecStart` word: quoted when it must be, with systemd's `%`
/// specifiers and `$` expansion escaped.
pub fn exec_word(path: &Path) -> Result<String, String> {
    let s = path
        .to_str()
        .ok_or_else(|| format!("{} is not UTF-8", path.display()))?;
    if s.chars().any(char::is_control) {
        return Err(format!("{s:?} has a control character"));
    }
    let escaped = s.replace('%', "%%").replace('$', "$$");
    if !escaped
        .chars()
        .any(|c| c.is_whitespace() || matches!(c, '"' | '\'' | '\\' | ';'))
    {
        return Ok(escaped);
    }
    Ok(format!(
        "\"{}\"",
        escaped.replace('\\', "\\\\").replace('"', "\\\"")
    ))
}

/// The unit for `binary`. `--state` and `--config` are spelled out: the user
/// manager need not share your shell's `XDG_*` variables, and `enroll` used
/// them.
pub fn render_unit(binary: &Path, layout: &Layout, linger_ours: bool) -> Result<String, String> {
    if !is_absolute(binary) {
        return Err(format!("{} is not an absolute path", binary.display()));
    }
    let exec = format!(
        "ExecStart={} serve --state {} --config {}",
        exec_word(binary)?,
        exec_word(&layout.state)?,
        exec_word(&layout.config)?
    );
    let unit = TEMPLATE.replace(EXEC_START, &exec);
    Ok(if linger_ours {
        format!("{LINGER_MARKER}\n{unit}")
    } else {
        unit
    })
}

pub fn linger_ours(unit: &str) -> bool {
    unit.lines().any(|l| l == LINGER_MARKER)
}

/// `loginctl show-user --property=Linger --value`'s answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Linger {
    On,
    Off,
    Unknown,
}

impl Linger {
    pub fn parse(out: &str) -> Self {
        match out.trim() {
            "yes" => Self::On,
            "no" => Self::Off,
            _ => Self::Unknown,
        }
    }
}

/// What a person runs when polkit will not let them change their own linger.
/// Storm never runs sudo.
pub fn linger_command(enable: bool, user: &str) -> String {
    let verb = if enable { "enable" } else { "disable" };
    format!("sudo loginctl {verb}-linger {user}")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallPlan {
    /// What `ExecStart` runs.
    pub binary: PathBuf,
    /// Copy this process's binary there first.
    pub copy: bool,
    /// Write a fresh `runtime.toml` naming [`Layout::workspaces`].
    pub write_config: bool,
    /// Try `loginctl enable-linger`.
    pub enable_linger: bool,
    /// The unit already says linger is Storm's.
    pub linger_ours: bool,
}

/// `exe` is this binary. One a package manager put in place (owned by
/// root) runs where it is, so the package's upgrades reach the unit; any
/// other (a tarball, a build) is copied to `~/.local/bin`.
pub fn plan_install(
    layout: &Layout,
    exe: &Path,
    exe_root_owned: bool,
    existing_unit: Option<&str>,
    config_exists: bool,
    linger: Linger,
    want_linger: bool,
) -> InstallPlan {
    let (binary, copy) = if exe == layout.bin || exe_root_owned {
        (exe.to_path_buf(), false)
    } else {
        (layout.bin.clone(), true)
    };
    InstallPlan {
        binary,
        copy,
        write_config: !config_exists,
        enable_linger: want_linger && linger != Linger::On,
        linger_ours: existing_unit.is_some_and(linger_ours),
    }
}

/// The `runtime.toml` `install --user` writes when there is none, and never
/// overwrites.
pub fn default_config(layout: &Layout) -> String {
    let path = |p: &Path| toml::Value::String(p.display().to_string()).to_string();
    format!(
        "# {config}: your own Runtime Host's configuration (decision 151).\n\
         # Written once by `storm-runtime install --user`, which never overwrites it;\n\
         # deploy/runtime.toml.example describes every key.\n\
         \n\
         # Each subdirectory of a root is a workspace, named by its directory name.\n\
         # A root may be anywhere you can write, your home included, but never in\n\
         # or around a Storm data root (D3).\n\
         workspace_roots = [{workspaces}]\n",
        config = layout.config.display(),
        workspaces = path(&layout.workspaces),
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UninstallPlan {
    /// `~/.local/bin/storm-runtime`, when the unit ran that copy.
    pub remove_binary: bool,
    pub disable_linger: bool,
}

pub fn plan_uninstall(layout: &Layout, existing_unit: Option<&str>) -> UninstallPlan {
    let ran_copy = exec_word(&layout.bin).is_ok_and(|bin| {
        existing_unit.is_some_and(|u| {
            u.lines()
                .any(|l| l.starts_with(&format!("ExecStart={bin} ")))
        })
    });
    UninstallPlan {
        remove_binary: ran_copy,
        disable_linger: existing_unit.is_some_and(linger_ours),
    }
}

/// The drop-in `storm-server up` writes: the account the packaged server
/// runs as (`User=`) and its data root (`ReadWritePaths=`). World-readable,
/// unlike `/etc/storm/storm.env`.
pub const SERVER_DROPIN: &str = "/etc/systemd/system/storm-server.service.d/data-root.conf";

/// The packaged server's data root when no drop-in moved it.
pub const SERVER_DATA: &str = "/srv/storm";

/// The packaged server's account and group.
pub const SERVER_GROUP: &str = "storm";

/// The last `key=`'s first word in a systemd unit or drop-in.
fn unit_value<'a>(unit: &'a str, key: &str) -> Option<&'a str> {
    unit.lines()
        .rev()
        .filter_map(|l| l.trim().strip_prefix(key)?.strip_prefix('='))
        .find_map(|v| v.split_whitespace().next())
}

/// The server's data root: the drop-in's first `ReadWritePaths`, else
/// [`SERVER_DATA`].
pub fn server_data_root(dropin: Option<&str>) -> PathBuf {
    dropin
        .and_then(|d| unit_value(d, "ReadWritePaths"))
        .unwrap_or(SERVER_DATA)
        .into()
}

/// Whether `user`, whose groups are `gids`, is in `group` by `/etc/group`'s
/// text: its gid among theirs, or their name on its line (a membership
/// added since they logged in).
pub fn in_group(group_file: &str, group: &str, user: &str, gids: &[u32]) -> bool {
    group_file.lines().any(|l| {
        let f: Vec<&str> = l.split(':').collect();
        f.len() >= 4
            && f[0] == group
            && (f[2].parse().is_ok_and(|g: u32| gids.contains(&g))
                || f[3].split(',').any(|m| m.trim() == user))
    })
}

/// What `install --user` found of a Storm server on this machine.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ServerFacts {
    /// [`SERVER_DROPIN`]'s text, if it is there.
    pub dropin: Option<String>,
    /// The server's data root, or its `state`, belongs to this account.
    pub data_ours: bool,
    /// A `storm-server` process runs as this account.
    pub running_ours: bool,
    /// This account is in [`SERVER_GROUP`].
    pub in_server_group: bool,
}

/// What `install --user` warns about before it installs (as on macOS and
/// Windows, a warning, never a refusal): a Storm server whose data this
/// account can read, so this host's agents could too.
pub fn server_notes(user: &str, facts: &ServerFacts) -> Vec<String> {
    let mut why = Vec::new();
    if facts.dropin.as_deref().and_then(|d| unit_value(d, "User")) == Some(user) {
        why.push(format!("{SERVER_DROPIN} runs it as {user}"));
    }
    if facts.data_ours {
        why.push(format!(
            "{} is yours",
            server_data_root(facts.dropin.as_deref()).display()
        ));
    }
    if facts.running_ours {
        why.push(format!("a storm-server process runs as {user}"));
    }
    let mut notes = Vec::new();
    if !why.is_empty() {
        notes.push(format!(
            "warning: a Storm server runs as you on this machine ({}), so this host's \
             agents can read its vaults on disk. To keep them out, run the host as its \
             own account instead: `sudo apt install storm-runtime`.",
            why.join("; ")
        ));
    }
    if facts.in_server_group {
        notes.push(format!(
            "warning: you are in the `{SERVER_GROUP}` group, which can read the packaged \
             server's files ({}), so this host's agents can too. Leave it \
             (`sudo gpasswd -d {user} {SERVER_GROUP}`, then log in again), or run the host \
             as its own account: `sudo apt install storm-runtime`.",
            server_data_root(facts.dropin.as_deref()).display()
        ));
    }
    notes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env<'a>(vars: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<OsString> + 'a {
        move |name| {
            vars.iter()
                .find(|(k, _)| *k == name)
                .map(|(_, v)| OsString::from(v))
        }
    }

    fn alice() -> Layout {
        layout(env(&[("HOME", "/home/alice")])).unwrap()
    }

    #[test]
    fn the_layout_is_the_persons_own() {
        let l = alice();
        assert_eq!(l.bin, Path::new("/home/alice/.local/bin/storm-runtime"));
        assert_eq!(
            l.unit,
            Path::new("/home/alice/.config/systemd/user/storm-runtime.service")
        );
        assert_eq!(
            l.wants,
            Path::new(
                "/home/alice/.config/systemd/user/default.target.wants/storm-runtime.service"
            )
        );
        assert_eq!(l.state, Path::new("/home/alice/.local/state/storm-runtime"));
        assert_eq!(
            l.config,
            Path::new("/home/alice/.config/storm-runtime/runtime.toml")
        );
        assert_eq!(
            l.workspaces,
            Path::new("/home/alice/.local/state/storm-runtime/workspaces")
        );
        let xdg = layout(env(&[
            ("HOME", "/home/alice"),
            ("XDG_CONFIG_HOME", "/cfg"),
            ("XDG_STATE_HOME", "/st"),
        ]))
        .unwrap();
        assert_eq!(
            xdg.unit,
            Path::new("/cfg/systemd/user/storm-runtime.service")
        );
        assert_eq!(xdg.config, Path::new("/cfg/storm-runtime/runtime.toml"));
        assert_eq!(xdg.state, Path::new("/st/storm-runtime"));
        let relative = layout(env(&[("HOME", "/home/alice"), ("XDG_CONFIG_HOME", "cfg")])).unwrap();
        assert_eq!(relative.unit, l.unit);
        assert!(layout(env(&[])).is_err());
    }

    #[test]
    fn a_tarball_is_copied_and_a_package_runs_in_place() {
        let l = alice();
        let fresh = |exe: &str, root: bool| {
            plan_install(&l, Path::new(exe), root, None, false, Linger::Off, true)
        };
        let tarball = fresh("/home/alice/Downloads/storm-runtime", false);
        assert_eq!(
            (tarball.binary.as_path(), tarball.copy),
            (l.bin.as_path(), true)
        );
        let package = fresh("/usr/bin/storm-runtime", true);
        assert_eq!(
            (package.binary.as_path(), package.copy),
            (Path::new("/usr/bin/storm-runtime"), false)
        );
        let again = fresh("/home/alice/.local/bin/storm-runtime", false);
        assert_eq!(
            (again.binary.as_path(), again.copy),
            (l.bin.as_path(), false)
        );
        assert!(tarball.write_config && tarball.enable_linger && !tarball.linger_ours);
    }

    #[test]
    fn linger_is_offered_once_and_remembered() {
        let l = alice();
        let exe = Path::new("/tmp/x/storm-runtime");
        let plan = |unit: Option<&str>, linger, want| {
            plan_install(&l, exe, false, unit, true, linger, want)
        };
        assert!(!plan(None, Linger::On, true).enable_linger);
        assert!(plan(None, Linger::Unknown, true).enable_linger);
        assert!(!plan(None, Linger::Off, false).enable_linger);
        assert!(!plan(None, Linger::Off, true).write_config);
        let ours = render_unit(&l.bin, &l, true).unwrap();
        assert!(plan(Some(&ours), Linger::On, true).linger_ours);
        let theirs = render_unit(&l.bin, &l, false).unwrap();
        assert!(!plan(Some(&theirs), Linger::On, true).linger_ours);
        assert_eq!(Linger::parse("yes\n"), Linger::On);
        assert_eq!(Linger::parse("no"), Linger::Off);
        assert_eq!(Linger::parse(""), Linger::Unknown);
        assert_eq!(
            linger_command(true, "alice"),
            "sudo loginctl enable-linger alice"
        );
        assert_eq!(
            linger_command(false, "alice"),
            "sudo loginctl disable-linger alice"
        );
    }

    #[test]
    fn the_unit_runs_the_persons_paths() {
        let l = alice();
        let unit = render_unit(&l.bin, &l, false).unwrap();
        assert!(unit.contains(
            "\nExecStart=/home/alice/.local/bin/storm-runtime serve \
             --state /home/alice/.local/state/storm-runtime \
             --config /home/alice/.config/storm-runtime/runtime.toml\n"
        ));
        assert!(!unit.contains("@EXEC_START@"));
        assert!(!unit.contains("--exclusive-account"));
        assert!(!linger_ours(&unit));
        assert!(render_unit(Path::new("storm-runtime"), &l, false).is_err());
    }

    #[test]
    fn exec_words_are_quoted_and_escaped() {
        let w = |s: &str| exec_word(Path::new(s)).unwrap();
        assert_eq!(w("/home/a/bin/storm-runtime"), "/home/a/bin/storm-runtime");
        assert_eq!(w("/home/a b/x"), "\"/home/a b/x\"");
        assert_eq!(w("/home/100%/x"), "/home/100%%/x");
        assert_eq!(w("/home/$USER/x"), "/home/$$USER/x");
        assert_eq!(w(r#"/home/a"b\c"#), r#""/home/a\"b\\c""#);
        assert!(exec_word(Path::new("/home/a\nb")).is_err());
    }

    #[test]
    fn the_default_config_names_the_workspace_root() {
        let l = alice();
        let config: crate::config::RuntimeConfig = toml::from_str(&default_config(&l)).unwrap();
        assert_eq!(config.workspace_roots, std::slice::from_ref(&l.workspaces));
    }

    #[test]
    fn uninstall_removes_only_what_install_made() {
        let l = alice();
        let copied = render_unit(&l.bin, &l, true).unwrap();
        assert_eq!(
            plan_uninstall(&l, Some(&copied)),
            UninstallPlan {
                remove_binary: true,
                disable_linger: true
            }
        );
        let package = render_unit(Path::new("/usr/bin/storm-runtime"), &l, false).unwrap();
        assert_eq!(
            plan_uninstall(&l, Some(&package)),
            UninstallPlan {
                remove_binary: false,
                disable_linger: false
            }
        );
        assert_eq!(
            plan_uninstall(&l, None),
            UninstallPlan {
                remove_binary: false,
                disable_linger: false
            }
        );
    }

    #[test]
    fn a_server_that_runs_as_you_or_shares_its_group_is_warned_about() {
        assert!(server_notes("alice", &ServerFacts::default()).is_empty());
        let packaged = ServerFacts {
            dropin: Some("[Service]\nUser=storm\nGroup=storm\nReadWritePaths=/srv/storm\n".into()),
            ..ServerFacts::default()
        };
        assert!(server_notes("alice", &packaged).is_empty());

        let mine = ServerFacts {
            dropin: Some(
                "[Unit]\nRequiresMountsFor=/home/alice/storm\n[Service]\nUser=alice\n\
                 Group=alice\nReadWritePaths=/home/alice/storm /nas/vaults\n"
                    .into(),
            ),
            data_ours: true,
            ..ServerFacts::default()
        };
        let notes = server_notes("alice", &mine);
        assert_eq!(notes.len(), 1, "{notes:?}");
        assert!(notes[0].contains("runs it as alice"), "{notes:?}");
        assert!(notes[0].contains("/home/alice/storm is yours"), "{notes:?}");
        assert!(notes[0].contains("can read its vaults"), "{notes:?}");

        let by_hand = ServerFacts {
            running_ours: true,
            in_server_group: true,
            ..ServerFacts::default()
        };
        let notes = server_notes("alice", &by_hand);
        assert_eq!(notes.len(), 2, "{notes:?}");
        assert!(notes[0].contains("a storm-server process runs as alice"));
        assert!(notes[1].contains("`storm` group"), "{notes:?}");
        assert!(notes[1].contains("(/srv/storm)"), "{notes:?}");
        assert!(
            notes[1].contains("sudo gpasswd -d alice storm"),
            "{notes:?}"
        );
    }

    #[test]
    fn group_membership_is_by_gid_or_by_name() {
        let group = "root:x:0:\nstorm:x:991:bob, alice\nstormy:x:992:carol\n";
        assert!(in_group(group, "storm", "carol", &[1000, 991]));
        assert!(in_group(group, "storm", "alice", &[1000]));
        assert!(!in_group(group, "storm", "carol", &[1000, 992]));
        assert!(!in_group("", "storm", "alice", &[991]));
        assert_eq!(server_data_root(None), Path::new("/srv/storm"));
    }
}
