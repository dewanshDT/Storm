//! The Windows logon task `install --user` registers (decision 151 E3): the
//! host as the person, in their own session, from a Scheduled Task they
//! register themselves, with no administrator. The `StormRuntime` service
//! ([`super::winsvc`]) stays the service install.
//!
//! Pure functions over strings, compiled on every platform, so the Linux
//! suite tests them. The task definition follows `storm-server up`'s
//! (`apps/server/src/install_windows.rs`). Executing it (`schtasks`) is
//! Windows' `windows_service` module.

use super::wincmd::push_arg;

/// The task's name in Task Scheduler's root folder.
pub const TASK: &str = "Storm Runtime Host";

/// Task Scheduler's `NORMAL_PRIORITY_CLASS`. Its default, 7, would run the
/// host and every agent below normal priority.
const PRIORITY: u8 = 5;

/// Where `install --user` puts a host a person runs as themselves. `state`
/// and `config` are N2's per-user paths, so `enroll` and `check` with no
/// flags find the same host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserLayout {
    pub profile: String,
    pub root: String,
    pub bin_dir: String,
    pub bin: String,
    pub config: String,
    pub state: String,
    pub logs: String,
    pub log_file: String,
    /// `storm-server up`'s record: a Storm server runs as this person.
    pub server_record: String,
}

impl UserLayout {
    /// For a person whose `%LOCALAPPDATA%` and `%USERPROFILE%` are these.
    pub fn for_profile(local_app_data: &str, user_profile: &str) -> Self {
        let local = local_app_data.trim_end_matches('\\');
        let root = format!(r"{local}\StormRuntime");
        let bin_dir = format!(r"{root}\bin");
        let logs = format!(r"{root}\logs");
        Self {
            profile: user_profile.trim_end_matches('\\').to_string(),
            bin: format!(r"{bin_dir}\storm-runtime.exe"),
            config: format!(r"{root}\runtime.toml"),
            state: format!(r"{root}\state"),
            log_file: format!(r"{logs}\storm-runtime.log"),
            server_record: format!(r"{local}\Storm\server.json"),
            root,
            bin_dir,
            logs,
        }
    }

    pub fn enrolled_marker(&self) -> String {
        format!(r"{}\{}", self.state, crate::identity::HOST_FILE)
    }

    /// Where the definition is written for `schtasks /create /xml`.
    pub fn scratch(&self) -> String {
        format!(r"{}\task.xml", self.root)
    }
}

fn xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// The task: at `user`'s logon, as `user`, with no elevation, only while
/// they are logged on (`InteractiveToken`), never stopped for its age, and
/// restarted every minute if it fails. `conhost` runs the host `--headless`:
/// a console of its own, with no window, which every console program the
/// host starts shares rather than opening one.
pub fn task_xml(layout: &UserLayout, user: &str, conhost: &str) -> String {
    let mut line = String::from("--headless");
    push_arg(&mut line, &layout.bin, true);
    for arg in [
        "serve",
        "--state",
        &layout.state,
        "--config",
        &layout.config,
        "--log-file",
        &layout.log_file,
    ] {
        push_arg(&mut line, arg, false);
    }
    format!(
        r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo>
    <Description>{description}</Description>
  </RegistrationInfo>
  <Triggers>
    <LogonTrigger>
      <Enabled>true</Enabled>
      <UserId>{user}</UserId>
    </LogonTrigger>
  </Triggers>
  <Principals>
    <Principal id="Author">
      <UserId>{user}</UserId>
      <LogonType>InteractiveToken</LogonType>
      <RunLevel>LeastPrivilege</RunLevel>
    </Principal>
  </Principals>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <AllowHardTerminate>true</AllowHardTerminate>
    <StartWhenAvailable>true</StartWhenAvailable>
    <RunOnlyIfNetworkAvailable>false</RunOnlyIfNetworkAvailable>
    <IdleSettings>
      <StopOnIdleEnd>false</StopOnIdleEnd>
      <RestartOnIdle>false</RestartOnIdle>
    </IdleSettings>
    <AllowStartOnDemand>true</AllowStartOnDemand>
    <Enabled>true</Enabled>
    <Hidden>false</Hidden>
    <RunOnlyIfIdle>false</RunOnlyIfIdle>
    <WakeToRun>false</WakeToRun>
    <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>
    <Priority>{PRIORITY}</Priority>
    <RestartOnFailure>
      <Interval>PT1M</Interval>
      <Count>999</Count>
    </RestartOnFailure>
  </Settings>
  <Actions Context="Author">
    <Exec>
      <Command>{command}</Command>
      <Arguments>{arguments}</Arguments>
      <WorkingDirectory>{working_dir}</WorkingDirectory>
    </Exec>
  </Actions>
</Task>
"#,
        description = xml("Storm's Runtime Host, as you, while you are logged on \
             (storm-runtime install --user)."),
        user = xml(user),
        command = xml(conhost),
        arguments = xml(&line),
        working_dir = xml(&layout.root),
    )
}

/// The `runtime.toml` `install --user` writes when there is none. It names
/// no workspace root: which of a person's folders an agent may work in is
/// theirs to say. TOML literal strings, so backslashes need no escaping.
pub fn user_default_config(layout: &UserLayout) -> String {
    format!(
        r#"# {config}: the configuration of the Storm Runtime Host you run as
# yourself (decision 151 E3). Every value here is an operational default.
# Written once by `storm-runtime install --user`, which never overwrites it.
# After editing it: schtasks /end /tn "{TASK}"; schtasks /run /tn "{TASK}"

# Each subdirectory of a root is a workspace, named by its directory name.
# There is none until you name one. A root may be in your profile.
# workspace_roots = ['{profile}\Code']

max_sessions = 8
scrollback_bytes = 4194304

# Where agent CLIs are looked for, and the PATH every session gets.
# Unset: your own bin directories, then the machine PATH.
# path = ['C:\Program Files\nodejs', 'C:\Windows\System32']
"#,
        config = layout.config,
        profile = layout.profile,
    )
}

/// What `install --user` found before acting.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UserFacts {
    /// Who registers the task, `DOMAIN\name`.
    pub user: Option<String>,
    /// Running elevated: the service's context (N2), whose paths differ.
    pub elevated: bool,
    /// Running as the service's own account.
    pub service_account: bool,
    pub config_exists: bool,
    pub enrolled: bool,
    pub service_installed: bool,
    /// A Storm server runs as this person (`storm-server up`'s record).
    pub server_here: bool,
}

/// One thing `install --user` does, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UserStep {
    /// A directory only the person (and SYSTEM and Administrators) can open.
    PrivateDir(String),
    /// End the task and the host it started: a running host holds its binary.
    EndTask,
    /// This binary, copied to `bin`.
    Binary,
    Config,
    /// `schtasks /create /xml` with [`task_xml`] for `user`.
    Register {
        user: String,
    },
    /// Off until enrolling turns it on, so an unenrolled host never runs.
    Disable,
    Run,
}

/// What `install --user` does on a Windows machine where it found `facts`,
/// and what it says first; or why it refuses.
pub fn plan_user_install(
    layout: &UserLayout,
    facts: &UserFacts,
) -> Result<(Vec<UserStep>, Vec<String>), String> {
    if facts.elevated {
        return Err(
            "install --user installs the host as you: run it in a PowerShell that is \
             not elevated (Run as administrator is `storm-runtime install`, the service)"
                .into(),
        );
    }
    if facts.service_account {
        return Err(format!(
            "install --user runs a host as a person; this is the service's own account {}",
            super::winsvc::ACCOUNT
        ));
    }
    let user = facts
        .user
        .clone()
        .ok_or("install --user: this process's account has no name to register the task for")?;
    let mut notes = Vec::new();
    if facts.service_installed {
        notes.push(format!(
            "The {} service is installed too: it is a separate host, as {}, with its own \
             state. `storm-runtime uninstall`, as administrator, removes it.",
            super::winsvc::SERVICE_NAME,
            super::winsvc::ACCOUNT
        ));
    }
    if facts.server_here {
        notes.push(
            "A Storm server runs as you on this PC, so this host's agents can read its \
             vaults on disk. To keep them out, run the host as its own account instead: \
             `storm-runtime install` as administrator."
                .into(),
        );
    }
    let mut steps = vec![
        UserStep::PrivateDir(layout.root.clone()),
        UserStep::PrivateDir(layout.bin_dir.clone()),
        UserStep::PrivateDir(layout.state.clone()),
        UserStep::PrivateDir(layout.logs.clone()),
        UserStep::EndTask,
        UserStep::Binary,
    ];
    if !facts.config_exists {
        steps.push(UserStep::Config);
    }
    steps.push(UserStep::Register { user });
    steps.push(if facts.enrolled {
        UserStep::Run
    } else {
        UserStep::Disable
    });
    Ok((steps, notes))
}

/// `s` as one PowerShell word: the per-user paths may have a space.
pub fn ps_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ada() -> UserLayout {
        UserLayout::for_profile(
            r"C:\Users\Ada Lovelace\AppData\Local\",
            r"C:\Users\Ada Lovelace",
        )
    }

    const CONHOST: &str = r"C:\Windows\System32\conhost.exe";

    fn element<'a>(x: &'a str, name: &str) -> &'a str {
        let open = format!("<{name}>");
        let start = x.find(&open).unwrap_or_else(|| panic!("no {name}")) + open.len();
        &x[start..start + x[start..].find(&format!("</{name}>")).unwrap()]
    }

    fn unxml(s: &str) -> String {
        s.replace("&quot;", "\"")
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&amp;", "&")
    }

    /// What `CommandLineToArgvW` gives back for the lines `push_arg` writes.
    fn split(line: &str) -> Vec<String> {
        let mut args = Vec::new();
        let mut cur = String::new();
        let (mut quoted, mut any) = (false, false);
        let chars: Vec<char> = line.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            match chars[i] {
                '\\' => {
                    let n = chars[i..].iter().take_while(|&&c| c == '\\').count();
                    i += n;
                    if chars.get(i) == Some(&'"') {
                        cur.extend(std::iter::repeat_n('\\', n / 2));
                        if n % 2 == 1 {
                            cur.push('"');
                            i += 1;
                        }
                    } else {
                        cur.extend(std::iter::repeat_n('\\', n));
                    }
                    any = true;
                    continue;
                }
                '"' => {
                    quoted = !quoted;
                    any = true;
                }
                ' ' if !quoted => {
                    if any {
                        args.push(std::mem::take(&mut cur));
                        any = false;
                    }
                }
                c => {
                    cur.push(c);
                    any = true;
                }
            }
            i += 1;
        }
        if any {
            args.push(cur);
        }
        args
    }

    #[test]
    fn the_user_layout_is_n2s_per_user_paths() {
        let l = ada();
        let n2 = super::super::paths::resolve(super::super::paths::Os::Windows, false, |n| {
            (n == "LOCALAPPDATA").then(|| r"C:\Users\Ada Lovelace\AppData\Local".into())
        })
        .unwrap();
        assert_eq!(std::path::PathBuf::from(&l.state), n2.state);
        assert_eq!(std::path::PathBuf::from(&l.config), n2.config);
        assert_eq!(
            l.bin,
            r"C:\Users\Ada Lovelace\AppData\Local\StormRuntime\bin\storm-runtime.exe"
        );
        assert_eq!(
            l.log_file,
            r"C:\Users\Ada Lovelace\AppData\Local\StormRuntime\logs\storm-runtime.log"
        );
        assert_eq!(
            l.enrolled_marker(),
            r"C:\Users\Ada Lovelace\AppData\Local\StormRuntime\state\host.json"
        );
        assert_eq!(
            l.server_record,
            r"C:\Users\Ada Lovelace\AppData\Local\Storm\server.json"
        );
    }

    #[test]
    fn the_task_runs_serve_on_the_per_user_paths_at_logon_as_the_person() {
        let l = ada();
        let x = task_xml(&l, r"PC\Ada", CONHOST);
        assert!(x.starts_with(r#"<?xml version="1.0" encoding="UTF-16"?>"#));
        assert_eq!(element(element(&x, "LogonTrigger"), "UserId"), r"PC\Ada");
        let principal = element(&x, "Principals");
        assert!(principal.contains(r"<UserId>PC\Ada</UserId>"));
        assert!(principal.contains("<LogonType>InteractiveToken</LogonType>"));
        assert!(principal.contains("<RunLevel>LeastPrivilege</RunLevel>"));
        assert_eq!(element(&x, "ExecutionTimeLimit"), "PT0S");
        assert_eq!(element(element(&x, "RestartOnFailure"), "Interval"), "PT1M");
        assert_eq!(element(&x, "Priority"), "5");
        assert_eq!(element(&x, "Command"), CONHOST);
        assert_eq!(element(&x, "WorkingDirectory"), l.root);
        // Each path is one argument, space and all; the service's flag never.
        assert_eq!(
            split(&unxml(element(&x, "Arguments"))),
            [
                "--headless",
                l.bin.as_str(),
                "serve",
                "--state",
                l.state.as_str(),
                "--config",
                l.config.as_str(),
                "--log-file",
                l.log_file.as_str(),
            ]
        );
        assert!(!x.contains("--exclusive-account"));
        assert!(!x.contains("HighestAvailable"));
    }

    /// The same definition as `storm-server up`'s logon task, but for the
    /// action and its priority.
    #[test]
    fn the_task_matches_the_servers_where_it_applies() {
        let server = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../server/src/install_windows.rs"),
        )
        .unwrap();
        let template = |text: &str| -> String {
            let start = text.find("  <Principals>").unwrap();
            let end = text.find("  </Settings>").unwrap();
            text[start..end]
                .lines()
                .filter(|l| !l.trim_start().starts_with("<Priority>"))
                .collect::<Vec<_>>()
                .join("\n")
        };
        let ours = task_xml(&ada(), "{user}", CONHOST);
        assert_eq!(template(&ours), template(&server));
        assert!(server.contains(
            "    <LogonTrigger>\\n      <Enabled>true</Enabled>\\n      <UserId>{}</UserId>\\n    </LogonTrigger>\\n"
        ));
        assert!(ours.contains(
            "    <LogonTrigger>\n      <Enabled>true</Enabled>\n      <UserId>{user}</UserId>\n    </LogonTrigger>\n"
        ));
        assert!(server.contains(r#"<?xml version="1.0" encoding="UTF-16"?>"#));
        assert!(server.contains(r#"<Actions Context="Author">"#));
    }

    #[test]
    fn a_principal_or_path_cannot_break_the_xml() {
        let l = UserLayout::for_profile(r"C:\Users\a&b<c>\AppData\Local", r"C:\Users\a&b<c>");
        let x = task_xml(&l, "a<b>&\"", CONHOST);
        assert!(x.contains("<UserId>a&lt;b&gt;&amp;&quot;</UserId>"));
        assert!(x.contains(r"<WorkingDirectory>C:\Users\a&amp;b&lt;c&gt;\AppData"));
        assert!(!x.contains("a&b"));
    }

    #[test]
    fn the_default_config_parses_and_names_no_root() {
        let l = ada();
        let text = user_default_config(&l);
        let config: crate::config::RuntimeConfig = toml::from_str(&text).unwrap();
        assert!(config.workspace_roots.is_empty());
        assert!(config.path.is_none());
        assert!(text.contains(r"# workspace_roots = ['C:\Users\Ada Lovelace\Code']"));
    }

    fn facts() -> UserFacts {
        UserFacts {
            user: Some(r"PC\Ada".into()),
            ..UserFacts::default()
        }
    }

    #[test]
    fn a_fresh_install_registers_the_task_and_leaves_it_off_until_enrolled() {
        let l = ada();
        let (steps, notes) = plan_user_install(&l, &facts()).unwrap();
        assert_eq!(
            steps,
            [
                UserStep::PrivateDir(l.root.clone()),
                UserStep::PrivateDir(l.bin_dir.clone()),
                UserStep::PrivateDir(l.state.clone()),
                UserStep::PrivateDir(l.logs.clone()),
                UserStep::EndTask,
                UserStep::Binary,
                UserStep::Config,
                UserStep::Register {
                    user: r"PC\Ada".into()
                },
                UserStep::Disable,
            ]
        );
        assert!(notes.is_empty());
    }

    #[test]
    fn an_upgrade_keeps_the_config_and_runs_the_enrolled_host() {
        let (steps, _) = plan_user_install(
            &ada(),
            &UserFacts {
                config_exists: true,
                enrolled: true,
                ..facts()
            },
        )
        .unwrap();
        assert!(!steps.contains(&UserStep::Config));
        assert!(!steps.contains(&UserStep::Disable));
        assert_eq!(steps.last(), Some(&UserStep::Run));
        let end = steps.iter().position(|s| *s == UserStep::EndTask).unwrap();
        let bin = steps.iter().position(|s| *s == UserStep::Binary).unwrap();
        assert!(end < bin, "a running host holds its binary");
    }

    #[test]
    fn elevated_or_as_the_service_it_refuses() {
        let elevated = plan_user_install(
            &ada(),
            &UserFacts {
                elevated: true,
                ..facts()
            },
        )
        .unwrap_err();
        assert!(elevated.contains("not elevated"), "{elevated}");
        let service = plan_user_install(
            &ada(),
            &UserFacts {
                service_account: true,
                ..facts()
            },
        )
        .unwrap_err();
        assert!(service.contains(r"NT SERVICE\StormRuntime"), "{service}");
        assert!(
            plan_user_install(
                &ada(),
                &UserFacts {
                    user: None,
                    ..facts()
                }
            )
            .is_err()
        );
    }

    #[test]
    fn it_says_when_the_service_or_a_server_is_here_too() {
        let (_, notes) = plan_user_install(
            &ada(),
            &UserFacts {
                service_installed: true,
                server_here: true,
                ..facts()
            },
        )
        .unwrap();
        assert_eq!(notes.len(), 2);
        assert!(notes[0].contains("StormRuntime service"));
        assert!(notes[1].contains("vaults"));
    }

    #[test]
    fn powershell_quoting() {
        assert_eq!(ps_quote(r"C:\Users\O'Neil\x"), r"'C:\Users\O''Neil\x'");
    }
}
