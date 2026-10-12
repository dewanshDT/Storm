//! The Windows service definition (D16, AM48–AM50, AM53, AM54): the layout
//! under `C:\ProgramData\StormRuntime`, the DACLs `install` sets, the
//! service's environment, the default `PATH` and the rules for workspace
//! roots.
//!
//! Pure functions over strings, compiled on every platform, so the Linux
//! suite tests them too. Windows paths are strings here because a `Path` on
//! Linux would not split them. Executing them is Windows' `windows_service`
//! module.

/// The service's name, and so the virtual account's (AM49).
pub const SERVICE_NAME: &str = "StormRuntime";

pub const DISPLAY_NAME: &str = "Storm Runtime Host";

/// The virtual service account Windows creates with the service (AM49).
pub const ACCOUNT: &str = r"NT SERVICE\StormRuntime";

/// The host's state directory on Windows: `platform::DEFAULT_STATE` there.
pub const STATE: &str = r"C:\ProgramData\StormRuntime\state";

/// The operator's `runtime.toml` on Windows: `platform::DEFAULT_CONFIG` there.
pub const CONFIG: &str = r"C:\ProgramData\StormRuntime\runtime.toml";

/// `LocalSystem` and `BUILTIN\Administrators`, by SID so a localized Windows
/// spells them the same.
pub const SYSTEM_SID: &str = "*S-1-5-18";
pub const ADMINISTRATORS_SID: &str = "*S-1-5-32-544";

/// The exit code a revoked host stops with (AM36): service-specific, so the
/// recovery actions never restart it.
pub const REVOKED_EXIT: u32 = 3;

/// Where everything a Windows host owns lives (AM48). Only the binary's
/// path has a space, and no agent build tool sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    pub root: String,
    pub bin_dir: String,
    pub bin: String,
    pub config: String,
    pub state: String,
    pub home: String,
    pub workspaces: String,
    pub logs: String,
    pub log_file: String,
}

impl Layout {
    pub fn standard() -> Self {
        Self::under(r"C:\ProgramData", r"C:\Program Files")
    }

    /// The layout for a `%ProgramData%` and `%ProgramFiles%` that may not be
    /// on `C:`.
    pub fn under(program_data: &str, program_files: &str) -> Self {
        let root = format!(r"{}\StormRuntime", program_data.trim_end_matches('\\'));
        let bin_dir = format!(r"{}\StormRuntime", program_files.trim_end_matches('\\'));
        let state = format!(r"{root}\state");
        let logs = format!(r"{root}\logs");
        Self {
            bin: format!(r"{bin_dir}\storm-runtime.exe"),
            config: format!(r"{root}\runtime.toml"),
            home: format!(r"{state}\home"),
            workspaces: format!(r"{root}\workspaces"),
            log_file: format!(r"{logs}\storm-runtime.log"),
            root,
            bin_dir,
            state,
            logs,
        }
    }

    /// Every directory `install` creates, parents first.
    pub fn directories(&self) -> Vec<String> {
        let home = &self.home;
        vec![
            self.root.clone(),
            self.state.clone(),
            home.clone(),
            format!(r"{home}\.local\bin"),
            format!(r"{home}\AppData\Roaming"),
            format!(r"{home}\AppData\Local\Temp"),
            self.workspaces.clone(),
            self.logs.clone(),
        ]
    }
}

/// The `icacls` calls that give the layout its DACLs (AM48, AM54), in order.
/// `/inheritance:r` makes each DACL protected: `C:\ProgramData` lets every
/// user create files, and none of that may reach here. `operator` is who
/// shares the workspace root.
pub fn acl_plan(layout: &Layout, operator: &str) -> Vec<(String, Vec<String>)> {
    let full = |who: &str| format!("{who}:(OI)(CI)F");
    let modify = |who: &str| format!("{who}:(OI)(CI)M");
    let protected = |grants: Vec<String>| {
        let mut args = vec!["/inheritance:r".to_string(), "/grant:r".to_string()];
        args.extend(grants);
        args
    };
    let admin = || vec![full(SYSTEM_SID), full(ADMINISTRATORS_SID)];
    let with = |extra: Vec<String>| {
        let mut grants = admin();
        grants.extend(extra);
        protected(grants)
    };
    vec![
        // The root and the config file under it: the account reads, never writes.
        (
            layout.root.clone(),
            with(vec![format!("{ACCOUNT}:(OI)(CI)RX")]),
        ),
        (layout.state.clone(), with(vec![modify(ACCOUNT)])),
        (
            layout.workspaces.clone(),
            with(vec![modify(ACCOUNT), modify(operator)]),
        ),
        (layout.logs.clone(), with(vec![modify(ACCOUNT)])),
    ]
}

/// The service's registry `Environment` (AM50, AM53): the account's profile
/// variables, all under `state\home`, so agent CLIs keep their logins and
/// caches there. `PATH` is not here: the host computes it when it starts.
pub fn service_environment(layout: &Layout) -> Vec<String> {
    let home = &layout.home;
    let temp = format!(r"{home}\AppData\Local\Temp");
    vec![
        format!("HOME={home}"),
        format!("USERPROFILE={home}"),
        format!(r"APPDATA={home}\AppData\Roaming"),
        format!(r"LOCALAPPDATA={home}\AppData\Local"),
        format!("TEMP={temp}"),
        format!("TMP={temp}"),
    ]
}

/// The default `PATH` (AM53): what the account installed for itself, then
/// the machine's `PATH` as the registry holds it now. The SCM's own copy is
/// frozen at boot.
pub fn path_for_home(home: &str, machine_path: &str) -> String {
    let mut parts = vec![
        format!(r"{home}\.local\bin"),
        format!(r"{home}\AppData\Roaming\npm"),
    ];
    parts.extend(
        machine_path
            .split(';')
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .map(String::from),
    );
    parts.join(";")
}

/// The `safe.directory` value for every workspace, in git's own spelling
/// (forward slashes): AM54's counterpart of macOS's.
pub fn git_safe_directory(layout: &Layout) -> String {
    format!("{}/*", layout.workspaces.replace('\\', "/"))
}

/// The `runtime.toml` `install` writes when there is none. TOML literal
/// strings, so backslashes need no escaping.
pub fn default_config(layout: &Layout) -> String {
    format!(
        r#"# {config}: the Storm Runtime Host's configuration
# (freeze §5.7; Windows, D16). Every value here is an operational default.
# Written once by `storm-runtime install`, which never overwrites it.

# Each subdirectory of a root is a workspace, named by its directory name.
# You and the host's account can both read and write the default root (AM54).
# Roots under C:\Users, on a network share or on a removable drive are
# refused: to work on an existing checkout, clone or move it into a
# workspace here.
workspace_roots = ['{workspaces}']

max_sessions = 8
scrollback_bytes = 4194304

# Where agent CLIs are looked for, and the PATH every session gets (AM53).
# Unset: the account's own bin directories, then the machine PATH.
# path = ['C:\Program Files\nodejs', 'C:\Windows\System32']

# Storm data roots no workspace may overlap (D3).
forbidden_roots = []
"#,
        config = layout.config,
        workspaces = layout.workspaces,
    )
}

/// What kind of drive a root is on, as `GetDriveTypeW` says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriveKind {
    Fixed,
    Removable,
    Remote,
    Optical,
    Other,
}

/// `path` without a `\\?\` prefix, with `/` as `\` and no trailing `\` (a
/// drive root keeps its own).
pub fn normalize(path: &str) -> String {
    let path = path.replace('/', "\\");
    let path = if let Some(unc) = path.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{unc}")
    } else if let Some(local) = path.strip_prefix(r"\\?\") {
        local.to_string()
    } else {
        path
    };
    let trimmed = path.trim_end_matches('\\');
    if trimmed.len() == 2 && trimmed.ends_with(':') {
        format!("{trimmed}\\")
    } else {
        trimmed.to_string()
    }
}

fn components(path: &str) -> Vec<String> {
    normalize(path)
        .split('\\')
        .filter(|c| !c.is_empty())
        .map(str::to_lowercase)
        .collect()
}

/// Whether `a` is inside `b` or `b` inside `a`, comparing components and
/// ignoring case, as NTFS does.
pub fn overlaps(a: &str, b: &str) -> bool {
    let (a, b) = (components(a), components(b));
    let n = a.len().min(b.len());
    n > 0 && a[..n] == b[..n]
}

/// Why `root` may not be a workspace root (AM54), or `None`. `users` is
/// `%SystemDrive%\Users`; `drive` is what kind of drive `root` is on.
pub fn root_refusal(root: &str, users: &str, drive: DriveKind) -> Option<String> {
    let root = normalize(root);
    if root.starts_with(r"\\") {
        return Some("a network share".into());
    }
    match drive {
        DriveKind::Removable => return Some("a removable drive".into()),
        DriveKind::Remote => return Some("a network drive".into()),
        DriveKind::Optical => return Some("an optical drive".into()),
        DriveKind::Fixed | DriveKind::Other => {}
    }
    overlaps(&root, users).then(|| format!("{users}, where people's profiles are"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_layout_lives_in_program_data_with_the_binary_in_program_files() {
        let l = Layout::standard();
        assert_eq!(l.state, STATE);
        assert_eq!(l.config, CONFIG);
        assert_eq!(l.bin, r"C:\Program Files\StormRuntime\storm-runtime.exe");
        assert_eq!(l.home, r"C:\ProgramData\StormRuntime\state\home");
        assert_eq!(l.workspaces, r"C:\ProgramData\StormRuntime\workspaces");
        assert_eq!(
            l.log_file,
            r"C:\ProgramData\StormRuntime\logs\storm-runtime.log"
        );
        // Workspace paths reach agents' build tools.
        for p in [&l.state, &l.home, &l.workspaces, &l.config] {
            assert!(!p.contains(' '), "{p}");
        }
        let d = Layout::under(r"D:\Data\", r"D:\Apps");
        assert_eq!(d.state, r"D:\Data\StormRuntime\state");
        assert_eq!(d.bin, r"D:\Apps\StormRuntime\storm-runtime.exe");
    }

    #[test]
    fn every_directory_is_protected_and_only_the_workspaces_are_shared() {
        let l = Layout::standard();
        let plan = acl_plan(&l, r"PC\alice");
        assert_eq!(plan.len(), 4);
        for (path, args) in &plan {
            assert_eq!(args[0], "/inheritance:r", "{path} must not inherit");
            assert!(args.contains(&format!("{SYSTEM_SID}:(OI)(CI)F")), "{path}");
            assert!(
                args.contains(&format!("{ADMINISTRATORS_SID}:(OI)(CI)F")),
                "{path}"
            );
            let operator = args.iter().any(|a| a.starts_with(r"PC\alice"));
            assert_eq!(operator, *path == l.workspaces, "{path}: {args:?}");
            // Never a broad group.
            assert!(
                !args.iter().any(|a| a.contains("Users")
                    || a.contains("Everyone")
                    || a.contains("S-1-1-0")),
                "{path}: {args:?}"
            );
        }
        let root = &plan[0];
        assert_eq!(root.0, l.root);
        assert!(
            root.1.contains(&format!("{ACCOUNT}:(OI)(CI)RX")),
            "the account only reads the root"
        );
        let state = plan.iter().find(|(p, _)| *p == l.state).unwrap();
        assert!(state.1.contains(&format!("{ACCOUNT}:(OI)(CI)M")));
    }

    #[test]
    fn the_service_environment_roots_every_profile_variable_in_its_home() {
        let l = Layout::standard();
        let env = service_environment(&l);
        for var in [
            "HOME",
            "USERPROFILE",
            "APPDATA",
            "LOCALAPPDATA",
            "TEMP",
            "TMP",
        ] {
            let entry = env
                .iter()
                .find(|e| e.starts_with(&format!("{var}=")))
                .unwrap_or_else(|| panic!("{var} is missing"));
            assert!(entry.contains(&l.home), "{entry}");
        }
        assert!(!env.iter().any(|e| e.starts_with("PATH=")));
    }

    #[test]
    fn the_default_path_puts_the_accounts_own_bins_before_the_machines() {
        let p = path_for_home(r"C:\h", r"C:\Windows\system32; C:\Program Files\nodejs\;;");
        assert_eq!(
            p,
            r"C:\h\.local\bin;C:\h\AppData\Roaming\npm;C:\Windows\system32;C:\Program Files\nodejs\"
        );
    }

    #[test]
    fn git_gets_forward_slashes() {
        assert_eq!(
            git_safe_directory(&Layout::standard()),
            "C:/ProgramData/StormRuntime/workspaces/*"
        );
    }

    #[test]
    fn the_default_config_parses_and_names_the_default_root() {
        let l = Layout::standard();
        let config: crate::config::RuntimeConfig = toml::from_str(&default_config(&l)).unwrap();
        assert_eq!(
            config.workspace_roots,
            vec![std::path::PathBuf::from(&l.workspaces)]
        );
        assert!(config.forbidden_roots.is_empty());
    }

    #[test]
    fn paths_normalize_without_their_verbatim_prefix() {
        assert_eq!(normalize(r"\\?\C:\ProgramData\x\"), r"C:\ProgramData\x");
        assert_eq!(normalize(r"\\?\UNC\nas\share\x"), r"\\nas\share\x");
        assert_eq!(normalize("C:/src/app"), r"C:\src\app");
        assert_eq!(normalize(r"\\?\D:\"), r"D:\");
        assert_eq!(normalize("D:"), r"D:\");
    }

    #[test]
    fn roots_in_or_around_profiles_shares_and_removable_drives_are_refused() {
        let users = r"C:\Users";
        let refused = |root: &str, drive| root_refusal(root, users, drive);
        // Profiles, whatever the case or spelling.
        assert!(refused(r"C:\Users\alice\src", DriveKind::Fixed).is_some());
        assert!(refused(r"c:\users\Alice\OneDrive", DriveKind::Fixed).is_some());
        assert!(refused(r"\\?\C:\USERS\bob", DriveKind::Fixed).is_some());
        // Containing them.
        assert!(refused(r"C:\", DriveKind::Fixed).is_some());
        // Shares and drives that come and go.
        assert!(refused(r"\\nas\share\work", DriveKind::Other).is_some());
        assert!(refused(r"\\?\UNC\nas\share", DriveKind::Other).is_some());
        assert!(refused(r"E:\work", DriveKind::Removable).is_some());
        assert!(refused(r"Z:\work", DriveKind::Remote).is_some());
        // Storm's own root, and other fixed drives, are fine.
        assert_eq!(
            refused(r"C:\ProgramData\StormRuntime\workspaces", DriveKind::Fixed),
            None
        );
        assert_eq!(refused(r"D:\src", DriveKind::Fixed), None);
        // Components, not strings.
        assert_eq!(refused(r"C:\Users-shared\x", DriveKind::Fixed), None);
    }
}
