//! Where `enroll`, `check` and `serve` find the host's state and
//! `runtime.toml` when given no `--state` or `--config` (decision 151 N2).
//! The service account, and an administrator managing it, get the packaged
//! service's paths; anyone else gets paths of their own, which they own.
//!
//! Pure functions of the platform, the account and the environment, compiled
//! on every platform, so the Linux suite tests each platform's answer.
//! Each platform's paths are built as strings in its own separator: a `Path`
//! on Linux would not split a Windows path, nor one on Windows join a Unix one.

use std::ffi::OsString;
use std::path::PathBuf;

/// Which platform's layout to answer for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
    Linux,
    Macos,
    Windows,
}

/// The platform this binary runs on.
pub const OS: Os = if cfg!(target_os = "linux") {
    Os::Linux
} else if cfg!(target_os = "macos") {
    Os::Macos
} else {
    Os::Windows
};

/// Linux's `platform::DEFAULT_STATE`: the systemd unit's `StateDirectory`.
pub const LINUX_STATE: &str = "/var/lib/storm-runtime";

/// Linux's `platform::DEFAULT_CONFIG` (freeze §5.7).
pub const LINUX_CONFIG: &str = "/etc/storm-runtime/runtime.toml";

pub const LINUX_ACCOUNT: &str = "storm-runtime";

pub const MACOS_ACCOUNT: &str = "_stormruntime";

/// `rel` under `base`, joined with `/` whatever the platform.
pub fn unix_join(base: impl AsRef<std::ffi::OsStr>, rel: &str) -> PathBuf {
    let base = base.as_ref().to_string_lossy();
    format!("{}/{rel}", base.trim_end_matches('/')).into()
}

/// A host's state directory and `runtime.toml`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostPaths {
    pub state: PathBuf,
    pub config: PathBuf,
}

/// The packaged service's paths on `os`.
pub fn service_paths(os: Os) -> HostPaths {
    let (state, config) = match os {
        Os::Linux => (LINUX_STATE, LINUX_CONFIG),
        Os::Macos => (super::launchd::STATE, super::launchd::CONFIG),
        Os::Windows => (super::winsvc::STATE, super::winsvc::CONFIG),
    };
    HostPaths {
        state: state.into(),
        config: config.into(),
    }
}

/// The packaged service's account on `os`.
pub fn service_account(os: Os) -> &'static str {
    match os {
        Os::Linux => LINUX_ACCOUNT,
        Os::Macos => MACOS_ACCOUNT,
        Os::Windows => super::winsvc::ACCOUNT,
    }
}

/// Whether a process running as `account` acts for the packaged service:
/// it is the service account, or an administrator (root, or elevated on
/// Windows), who runs `enroll` and `check` against the service's state.
pub fn is_service(os: Os, account: Option<&str>, admin: bool) -> bool {
    admin
        || account.is_some_and(|a| match os {
            Os::Windows => a.eq_ignore_ascii_case(service_account(os)),
            _ => a == service_account(os),
        })
}

/// The paths for a process that acts for the service (`service`) or not,
/// with `env` its environment. Off the service: Linux's XDG state and config
/// homes; on macOS `~/Library/Application Support/StormRuntime`; on Windows
/// `%LOCALAPPDATA%\StormRuntime`, each holding `state` and `runtime.toml`
/// as the service's root does.
pub fn resolve(
    os: Os,
    service: bool,
    env: impl Fn(&str) -> Option<OsString>,
) -> Result<HostPaths, String> {
    if service {
        return Ok(service_paths(os));
    }
    let var = |name: &str| env(name).filter(|v| !v.is_empty());
    let home = || {
        var("HOME").map(PathBuf::from).ok_or_else(|| {
            "HOME is not set, so this account has no default state directory: \
             pass --state and --config"
                .to_string()
        })
    };
    match os {
        Os::Linux => {
            // The XDG spec ignores a relative value.
            let xdg = |name: &str, fallback: &str| match var(name) {
                Some(v) if v.to_string_lossy().starts_with('/') => Ok(PathBuf::from(v)),
                _ => home().map(|h| unix_join(h, fallback)),
            };
            Ok(HostPaths {
                state: unix_join(xdg("XDG_STATE_HOME", ".local/state")?, "storm-runtime"),
                config: unix_join(
                    xdg("XDG_CONFIG_HOME", ".config")?,
                    "storm-runtime/runtime.toml",
                ),
            })
        }
        Os::Macos => {
            let root = unix_join(home()?, "Library/Application Support/StormRuntime");
            Ok(HostPaths {
                state: unix_join(&root, "state"),
                config: unix_join(&root, "runtime.toml"),
            })
        }
        Os::Windows => {
            let local = var("LOCALAPPDATA").ok_or_else(|| {
                "LOCALAPPDATA is not set, so this account has no default state \
                 directory: pass --state and --config"
                    .to_string()
            })?;
            let root = format!(
                r"{}\StormRuntime",
                local.to_string_lossy().trim_end_matches('\\')
            );
            Ok(HostPaths {
                state: format!(r"{root}\state").into(),
                config: format!(r"{root}\runtime.toml").into(),
            })
        }
    }
}

/// The workspace root a fresh per-user `runtime.toml` names on every
/// platform: `workspaces` in the state directory.
pub fn user_workspaces(os: Os, state: &std::path::Path) -> PathBuf {
    match os {
        Os::Windows => format!(
            r"{}\workspaces",
            state.to_string_lossy().trim_end_matches('\\')
        )
        .into(),
        _ => unix_join(state, "workspaces"),
    }
}

/// What `uninstall --user --purge` removes from the state directory: every
/// entry but one that is, or holds, one of `keep` (the workspace roots).
/// Workspaces are never removed.
pub fn purge_targets(entries: &[PathBuf], keep: &[PathBuf]) -> Vec<PathBuf> {
    entries
        .iter()
        .filter(|e| !keep.iter().any(|r| r.starts_with(e)))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_platforms_default_workspace_root_is_in_its_state() {
        let home = env(&[
            ("HOME", "/home/alice"),
            ("LOCALAPPDATA", r"C:\Users\alice\AppData\Local"),
        ]);
        let ws = |os| user_workspaces(os, &resolve(os, false, &home).unwrap().state);
        assert_eq!(
            ws(Os::Linux),
            PathBuf::from("/home/alice/.local/state/storm-runtime/workspaces")
        );
        assert_eq!(
            ws(Os::Macos),
            PathBuf::from("/home/alice/Library/Application Support/StormRuntime/state/workspaces")
        );
        assert_eq!(
            ws(Os::Windows),
            PathBuf::from(r"C:\Users\alice\AppData\Local\StormRuntime\state\workspaces")
        );
    }

    #[test]
    fn a_purge_keeps_every_entry_that_holds_a_workspace_root() {
        let p = |s: &str| PathBuf::from(format!("/s/{s}"));
        assert_eq!(
            purge_targets(
                &[p("host.json"), p("workspaces"), p("agents"), p("mine")],
                &[p("workspaces"), p("mine/src"), PathBuf::from("/elsewhere")]
            ),
            [p("host.json"), p("agents")]
        );
    }

    fn env<'a>(vars: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<OsString> + 'a {
        move |name| {
            vars.iter()
                .find(|(k, _)| *k == name)
                .map(|(_, v)| OsString::from(v))
        }
    }

    fn paths(state: &str, config: &str) -> HostPaths {
        HostPaths {
            state: state.into(),
            config: config.into(),
        }
    }

    #[test]
    fn the_service_account_keeps_the_services_paths_on_every_platform() {
        let none = env(&[("HOME", "/home/alice"), ("LOCALAPPDATA", r"C:\x")]);
        for (os, state, config) in [
            (
                Os::Linux,
                "/var/lib/storm-runtime",
                "/etc/storm-runtime/runtime.toml",
            ),
            (
                Os::Macos,
                "/Library/StormRuntime/state",
                "/Library/StormRuntime/runtime.toml",
            ),
            (
                Os::Windows,
                r"C:\ProgramData\StormRuntime\state",
                r"C:\ProgramData\StormRuntime\runtime.toml",
            ),
        ] {
            assert!(is_service(os, Some(service_account(os)), false));
            assert_eq!(resolve(os, true, &none), Ok(paths(state, config)));
        }
        assert!(is_service(
            Os::Windows,
            Some(r"nt service\stormruntime"),
            false
        ));
        assert!(!is_service(Os::Linux, Some("STORM-RUNTIME"), false));
    }

    #[test]
    fn an_administrator_acts_for_the_service_and_a_person_does_not() {
        for os in [Os::Linux, Os::Macos, Os::Windows] {
            assert!(is_service(os, Some("root"), true));
            assert!(!is_service(os, Some("alice"), false));
            assert!(!is_service(os, None, false));
        }
        assert!(!is_service(Os::Linux, Some(MACOS_ACCOUNT), false));
    }

    #[test]
    fn a_person_on_linux_gets_xdg_paths() {
        let home = env(&[("HOME", "/home/alice")]);
        assert_eq!(
            resolve(Os::Linux, false, &home),
            Ok(paths(
                "/home/alice/.local/state/storm-runtime",
                "/home/alice/.config/storm-runtime/runtime.toml"
            ))
        );
        let xdg = env(&[
            ("HOME", "/home/alice"),
            ("XDG_STATE_HOME", "/data/state"),
            ("XDG_CONFIG_HOME", "/data/config"),
        ]);
        assert_eq!(
            resolve(Os::Linux, false, &xdg),
            Ok(paths(
                "/data/state/storm-runtime",
                "/data/config/storm-runtime/runtime.toml"
            ))
        );
        let relative = env(&[
            ("HOME", "/home/alice"),
            ("XDG_STATE_HOME", "state"),
            ("XDG_CONFIG_HOME", ""),
        ]);
        assert_eq!(
            resolve(Os::Linux, false, &relative),
            resolve(Os::Linux, false, &home)
        );
        let err = resolve(Os::Linux, false, env(&[])).unwrap_err();
        assert!(err.contains("--state"), "{err}");
    }

    #[test]
    fn a_person_on_macos_gets_application_support() {
        assert_eq!(
            resolve(Os::Macos, false, env(&[("HOME", "/Users/alice")])),
            Ok(paths(
                "/Users/alice/Library/Application Support/StormRuntime/state",
                "/Users/alice/Library/Application Support/StormRuntime/runtime.toml"
            ))
        );
        assert!(resolve(Os::Macos, false, env(&[("HOME", "")])).is_err());
    }

    #[test]
    fn a_person_on_windows_gets_local_app_data() {
        let local = env(&[("LOCALAPPDATA", r"C:\Users\alice\AppData\Local\")]);
        assert_eq!(
            resolve(Os::Windows, false, local),
            Ok(paths(
                r"C:\Users\alice\AppData\Local\StormRuntime\state",
                r"C:\Users\alice\AppData\Local\StormRuntime\runtime.toml"
            ))
        );
        assert!(resolve(Os::Windows, false, env(&[("HOME", "/x")])).is_err());
    }
}
