//! A session's own network (136d): a network namespace of its own, named
//! `storm-<session>` and owned by a unit of its own, which the session's unit
//! joins, with user-mode egress through `pasta` (passt's namespace mode)
//! running beside it in another unit, confined by Storm's AppArmor profile
//! (decision 140).
//!
//! Inside, the session has its own loopback and its own abstract-socket
//! space, so nothing another session or the host listens on there is
//! reachable. Outbound connections leave through `pasta` as ordinary
//! sockets on the host. The host's loopback is not mapped in (no gateway
//! mapping), nothing is forwarded into the namespace from outside, and from
//! the namespace to the host's loopback only the ports the operator lists
//! in `runtime.toml` (`isolation_host_ports`) are forwarded. Name
//! resolution is the one other way to the host: DNS sent to
//! [`DNS_FORWARD`] goes to the host's own resolver, which the session's
//! `/etc/resolv.conf` names.

use std::ffi::OsString;
use std::path::PathBuf;

use anyhow::{Result, bail};

use crate::units;

pub const PASTA: &str = "/usr/bin/pasta";
pub const IP: &str = "/usr/bin/ip";

/// The resolver address a session sees. Nothing listens there: `pasta`
/// sends what reaches its port 53 to the host's first nameserver.
pub const DNS_FORWARD: &str = "169.254.1.53";

/// More than this is not a list of services, and every port is a hole.
pub const MAX_HOST_PORTS: usize = 64;

pub fn netns_name(session: &str) -> String {
    format!("storm-{session}")
}

pub fn netns_path(session: &str) -> PathBuf {
    PathBuf::from("/run/netns").join(netns_name(session))
}

pub fn unit_name(session: &str) -> String {
    format!("storm-session-{session}-net.service")
}

/// The unit that owns the session's namespace, apart from pasta's: a
/// namespace cannot be deleted while a unit's mounts still hold it.
pub fn namespace_unit_name(session: &str) -> String {
    format!("storm-session-{session}-netns.service")
}

/// Storm's AppArmor profile for pasta (decision 140), which the network
/// unit names. Debian's own profile for pasta allows none of its paths.
pub const APPARMOR_PROFILE: &str = "storm-runtime-pasta";
pub const APPARMOR_PROFILE_FILE: &str = "/etc/apparmor.d/storm-runtime-pasta";

pub fn resolv_conf() -> String {
    format!("nameserver {DNS_FORWARD}\n")
}

/// The operator's list, as the helper uses it: sorted, once each, never
/// port 0, and short.
pub fn host_ports(list: &[u16]) -> Result<Vec<u16>> {
    if list.contains(&0) {
        bail!("isolation_host_ports: 0 is not a port");
    }
    let mut ports = list.to_vec();
    ports.sort_unstable();
    ports.dedup();
    if ports.len() > MAX_HOST_PORTS {
        bail!("isolation_host_ports: at most {MAX_HOST_PORTS} ports");
    }
    Ok(ports)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetSpec {
    pub session: String,
    /// Host loopback ports the session may reach, already checked.
    pub host_ports: Vec<u16>,
}

impl NetSpec {
    fn pid_dir(&self) -> String {
        format!("storm-session-{}-net", self.session)
    }

    /// The namespace as pasta opens it: bound into its own runtime
    /// directory, since /run/netns is root's alone (`0700`).
    fn netns_view(&self) -> String {
        format!("/run/{}/netns", self.pid_dir())
    }

    /// `pasta`'s arguments. Every forwarding default is `auto`, which
    /// would publish the session's listeners on the host and the host's
    /// listeners in the session, so each direction is set explicitly.
    pub fn pasta_args(&self) -> Vec<String> {
        let ports = if self.host_ports.is_empty() {
            "none".to_string()
        } else {
            let list: Vec<String> = self.host_ports.iter().map(u16::to_string).collect();
            list.join(",")
        };
        [
            "--quiet",
            "--config-net",
            "--netns",
            &self.netns_view(),
            "--pid",
            &format!("/run/{}/pasta.pid", self.pid_dir()),
            // The gateway would otherwise be the host's loopback.
            "--no-map-gw",
            "--dns-forward",
            DNS_FORWARD,
            "--tcp-ports",
            "none",
            "--udp-ports",
            "none",
            "--tcp-ns",
            &ports,
            "--udp-ns",
            "none",
        ]
        .map(str::to_string)
        .to_vec()
    }

    fn capabilities(&self) -> &'static str {
        // A listed port below 1024 is bound inside the namespace.
        if self.host_ports.iter().any(|&p| p < 1024) {
            "CAP_SYS_ADMIN CAP_NET_ADMIN CAP_NET_BIND_SERVICE"
        } else {
            "CAP_SYS_ADMIN CAP_NET_ADMIN"
        }
    }

    /// The network unit's properties, in order. Read by the tests as the
    /// spec.
    pub fn properties(&self) -> Vec<String> {
        let host = crate::HOST_UNIT;
        let namespace = namespace_unit_name(&self.session);
        let caps = self.capabilities();
        let p = |s: &str| s.to_string();
        vec![
            format!("Description=Storm session {} network", self.session),
            format!("BindsTo={host}"),
            format!("After={host}"),
            // Stopping the namespace's unit stops this one first, so the
            // namespace is deleted only once nothing holds a mount of it.
            format!("BindsTo={namespace}"),
            format!("After={namespace}"),
            // pasta returns once the namespace is configured, so the
            // session never starts before its network.
            p("Type=forking"),
            format!("PIDFile=/run/{}/pasta.pid", self.pid_dir()),
            format!("RuntimeDirectory={}", self.pid_dir()),
            p("RuntimeDirectoryMode=0700"),
            format!(
                "BindReadOnlyPaths={}:{}",
                netns_path(&self.session).display(),
                self.netns_view()
            ),
            // No `-`: on an AppArmor host a missing profile fails the unit
            // (systemd ignores it where AppArmor is off).
            format!("AppArmorProfile={APPARMOR_PROFILE}"),
            // pasta itself runs as a user of its own, with only what it
            // needs to enter and configure the namespace.
            p("DynamicUser=yes"),
            format!("CapabilityBoundingSet={caps}"),
            format!("AmbientCapabilities={caps}"),
            p("NoNewPrivileges=yes"),
            p("ProtectSystem=strict"),
            p("ProtectHome=yes"),
            p("PrivateTmp=yes"),
            p("PrivateIPC=yes"),
            p("DevicePolicy=closed"),
            p("DeviceAllow=/dev/net/tun rw"),
            p("ProtectKernelModules=yes"),
            p("ProtectKernelLogs=yes"),
            p("ProtectControlGroups=yes"),
            p("ProtectClock=yes"),
            p("ProtectHostname=yes"),
            p("ProtectProc=invisible"),
            p("LockPersonality=yes"),
            p("MemoryDenyWriteExecute=yes"),
            p("RestrictRealtime=yes"),
            p("RestrictSUIDSGID=yes"),
            p("RestrictAddressFamilies=AF_UNIX AF_INET AF_INET6 AF_NETLINK"),
            // pasta enters the session's namespace and sandboxes itself in
            // new mount, IPC, UTS and PID namespaces.
            p("RestrictNamespaces=ipc mnt net pid uts"),
            p("SystemCallArchitectures=native"),
            p("KeyringMode=private"),
            p("UMask=0077"),
        ]
    }

    pub fn systemd_run_args(&self) -> Vec<OsString> {
        let unit = unit_name(&self.session);
        units::run_args(&unit, false, self.properties(), self.exec())
    }

    /// The same unit as a file, for `systemd-analyze` offline.
    pub fn unit_file(&self) -> String {
        units::unit_file(self.properties(), &units::command_line(&self.exec()))
    }

    /// The namespace unit's properties. It runs only `ip`, as root, since
    /// a named namespace is a mount on the host's /run/netns.
    pub fn namespace_properties(&self) -> Vec<String> {
        let host = crate::HOST_UNIT;
        vec![
            format!(
                "Description=Storm session {} network namespace",
                self.session
            ),
            format!("BindsTo={host}"),
            format!("After={host}"),
            "Type=oneshot".to_string(),
            "RemainAfterExit=yes".to_string(),
            format!("ExecStop={IP} netns delete {}", netns_name(&self.session)),
        ]
    }

    /// Without `--wait`: systemd-run returns once the namespace exists,
    /// where `--wait` would block until the unit stops.
    pub fn namespace_run_args(&self) -> Vec<OsString> {
        let unit = namespace_unit_name(&self.session);
        units::run_args(
            &unit,
            false,
            self.namespace_properties(),
            self.namespace_exec(),
        )
    }

    pub fn namespace_unit_file(&self) -> String {
        units::unit_file(
            self.namespace_properties(),
            &units::command_line(&self.namespace_exec()),
        )
    }

    fn namespace_exec(&self) -> Vec<OsString> {
        [IP, "netns", "add", &netns_name(&self.session)]
            .map(OsString::from)
            .to_vec()
    }

    fn exec(&self) -> Vec<OsString> {
        let mut out = vec![OsString::from(PASTA)];
        out.extend(self.pasta_args().into_iter().map(OsString::from));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(ports: &[u16]) -> NetSpec {
        NetSpec {
            session: "ags_A1".into(),
            host_ports: ports.to_vec(),
        }
    }

    fn pair(args: &[String], flag: &str) -> String {
        let i = args.iter().position(|a| a == flag).expect(flag);
        args[i + 1].clone()
    }

    /// IT-4's half that is configuration: nothing of the host's loopback is
    /// mapped in but the listed ports, nothing is published from the
    /// session, and DNS goes to the host's resolver.
    #[test]
    fn it_4_only_the_listed_host_ports_and_dns_reach_the_host() {
        let args = spec(&[5432, 11434]).pasta_args();
        assert!(args.contains(&"--no-map-gw".to_string()));
        assert_eq!(pair(&args, "--tcp-ns"), "5432,11434");
        assert_eq!(pair(&args, "--udp-ns"), "none");
        assert_eq!(pair(&args, "--tcp-ports"), "none");
        assert_eq!(pair(&args, "--udp-ports"), "none");
        assert_eq!(pair(&args, "--dns-forward"), DNS_FORWARD);
        assert_eq!(
            pair(&args, "--netns"),
            "/run/storm-session-ags_A1-net/netns"
        );
        assert!(args.contains(&"--config-net".to_string()));
        assert_eq!(pair(&spec(&[]).pasta_args(), "--tcp-ns"), "none");
        assert_eq!(resolv_conf(), format!("nameserver {DNS_FORWARD}\n"));
    }

    /// Each flag pasta defaults to `auto` is given, so no default can open
    /// a way in or out.
    #[test]
    fn every_forwarding_direction_is_explicit() {
        let args = spec(&[]).pasta_args();
        for flag in ["--tcp-ports", "--udp-ports", "--tcp-ns", "--udp-ns"] {
            assert_eq!(args.iter().filter(|a| *a == flag).count(), 1, "{flag}");
        }
        for short in ["-t", "-u", "-T", "-U", "--map-host-loopback"] {
            assert!(!args.iter().any(|a| a == short), "{short}");
        }
    }

    #[test]
    fn the_network_unit_runs_pasta_unprivileged_but_for_two_capabilities() {
        let props = spec(&[5432]).properties();
        for want in [
            "Type=forking",
            "DynamicUser=yes",
            "CapabilityBoundingSet=CAP_SYS_ADMIN CAP_NET_ADMIN",
            "AmbientCapabilities=CAP_SYS_ADMIN CAP_NET_ADMIN",
            "NoNewPrivileges=yes",
            "BindsTo=storm-runtime.service",
            "After=storm-runtime.service",
            "BindsTo=storm-session-ags_A1-netns.service",
            "After=storm-session-ags_A1-netns.service",
            "PIDFile=/run/storm-session-ags_A1-net/pasta.pid",
            "RuntimeDirectory=storm-session-ags_A1-net",
            "RuntimeDirectoryMode=0700",
            "DeviceAllow=/dev/net/tun rw",
            "RestrictNamespaces=ipc mnt net pid uts",
            "AppArmorProfile=storm-runtime-pasta",
        ] {
            assert!(props.iter().any(|p| p == want), "{want} missing");
        }
        assert!(!props.iter().any(|p| p.starts_with("User=")));
        // Nothing of it runs as root: the namespace is its own unit's.
        assert!(!props.iter().any(|p| p.contains("=+") || p.contains("=-+")));
        assert!(!props.iter().any(|p| p.contains("netns add")
            || p.contains("netns delete")
            || p.starts_with("ExecStartPre=")
            || p.starts_with("ExecStopPost=")));
        let low = spec(&[80]).properties();
        assert!(low.contains(
            &"AmbientCapabilities=CAP_SYS_ADMIN CAP_NET_ADMIN CAP_NET_BIND_SERVICE".to_string()
        ));
    }

    /// Found on a Debian 13 host (2026-10-26): pasta, a dynamic user, could
    /// not open `/run/netns` (`0700 root`), and Debian's AppArmor profile for
    /// pasta allows none of the unit's paths. The unit sees exactly its own
    /// namespace file, in its own runtime directory, under Storm's profile.
    #[test]
    fn pasta_reaches_its_namespace_through_its_own_runtime_directory() {
        let props = spec(&[]).properties();
        let binds: Vec<&String> = props
            .iter()
            .filter(|p| {
                p.starts_with("BindPaths=")
                    || p.starts_with("BindReadOnlyPaths=")
                    || p.starts_with("TemporaryFileSystem=")
            })
            .collect();
        assert_eq!(
            binds,
            ["BindReadOnlyPaths=/run/netns/storm-ags_A1:/run/storm-session-ags_A1-net/netns"]
        );
        let profiles: Vec<&String> = props
            .iter()
            .filter(|p| p.starts_with("AppArmorProfile="))
            .collect();
        assert_eq!(profiles, ["AppArmorProfile=storm-runtime-pasta"], "no `-`");
        assert_eq!(
            pair(&spec(&[]).pasta_args(), "--netns"),
            "/run/storm-session-ags_A1-net/netns"
        );
    }

    /// The namespace outlives pasta's mounts of it: its own unit adds it
    /// and deletes it only once the network unit (bound to it) has stopped.
    #[test]
    fn the_namespace_is_its_own_units() {
        let s = spec(&[5432]);
        let props = s.namespace_properties();
        for want in [
            "Type=oneshot",
            "RemainAfterExit=yes",
            "BindsTo=storm-runtime.service",
            "After=storm-runtime.service",
            "ExecStop=/usr/bin/ip netns delete storm-ags_A1",
        ] {
            assert!(props.iter().any(|p| p == want), "{want} missing");
        }
        let args: Vec<String> = s
            .namespace_run_args()
            .into_iter()
            .map(|a| a.into_string().unwrap())
            .collect();
        assert_eq!(args[0], "--unit=storm-session-ags_A1-netns.service");
        // With --wait, systemd-run would block until the namespace is gone.
        assert!(!args.iter().any(|a| a == "--wait"));
        let dash = args.iter().position(|a| a == "--").unwrap();
        assert_eq!(
            &args[dash + 1..],
            ["/usr/bin/ip", "netns", "add", "storm-ags_A1"]
        );
        assert_eq!(
            namespace_unit_name("ags_A1"),
            "storm-session-ags_A1-netns.service"
        );
    }

    /// The profile `AppArmorProfile=` names, as shipped: pasta's own
    /// abstraction where passt provides one, and only the unit's own paths.
    #[test]
    fn the_apparmor_profile_allows_only_the_units_paths() {
        let text = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../../deploy/storm-runtime-pasta.apparmor"),
        )
        .unwrap();
        assert!(text.contains(&format!(
            "profile {APPARMOR_PROFILE} flags=(attach_disconnected)"
        )));
        assert!(text.contains("include if exists <abstractions/pasta>"));
        let rules: Vec<&str> = text
            .lines()
            .map(str::trim)
            .filter(|l| l.starts_with("/run/"))
            .collect();
        assert_eq!(
            rules,
            [
                "/run/storm-session-*-net/ r,",
                "/run/storm-session-*-net/netns r,",
                "/run/storm-session-*-net/pasta.pid rw,",
            ]
        );
    }

    #[test]
    fn systemd_run_starts_pasta_with_its_arguments() {
        let s = spec(&[5432]);
        let args: Vec<String> = s
            .systemd_run_args()
            .into_iter()
            .map(|a| a.into_string().unwrap())
            .collect();
        assert_eq!(args[0], "--unit=storm-session-ags_A1-net.service");
        let dash = args.iter().position(|a| a == "--").unwrap();
        assert_eq!(args[dash + 1], PASTA);
        assert_eq!(&args[dash + 2..], s.pasta_args());
        assert_eq!(
            args.iter().filter(|a| *a == "-p").count(),
            s.properties().len()
        );
    }

    #[test]
    fn the_operators_ports_are_checked() {
        assert_eq!(host_ports(&[11434, 5432, 5432]).unwrap(), [5432, 11434]);
        assert!(host_ports(&[0]).is_err());
        let many: Vec<u16> = (1..=65).collect();
        assert!(host_ports(&many).is_err());
        assert!(host_ports(&[]).unwrap().is_empty());
    }

    #[test]
    fn the_network_unit_verifies_offline() {
        let Some(analyze) = crate::units::tests::which("systemd-analyze") else {
            return;
        };
        let t = tempfile::tempdir().unwrap();
        let path = t.path().join("storm-session-ags_A1-net.service");
        std::fs::write(&path, spec(&[5432]).unit_file()).unwrap();
        let out = std::process::Command::new(analyze)
            .args(["verify", "--man=no", "--recursive-errors=no"])
            .arg(&path)
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&out.stderr);
        for line in stderr.lines() {
            assert!(
                !line.contains("Unknown key")
                    && !line.contains("Failed to parse")
                    && !line.contains("Invalid")
                    && !line.contains("Unknown section"),
                "{stderr}"
            );
        }
    }

    #[test]
    fn the_namespace_unit_verifies_offline() {
        let Some(analyze) = crate::units::tests::which("systemd-analyze") else {
            return;
        };
        let t = tempfile::tempdir().unwrap();
        let path = t.path().join("storm-session-ags_A1-netns.service");
        std::fs::write(&path, spec(&[]).namespace_unit_file()).unwrap();
        let out = std::process::Command::new(analyze)
            .args(["verify", "--man=no", "--recursive-errors=no"])
            .arg(&path)
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&out.stderr);
        for line in stderr.lines() {
            assert!(
                !line.contains("Unknown key")
                    && !line.contains("Failed to parse")
                    && !line.contains("Invalid")
                    && !line.contains("Unknown section"),
                "{stderr}"
            );
        }
    }

    /// The arguments against a real pasta, where one is given
    /// (`STORM_PASTA_BIN`) or installed: every option and value parses. It
    /// stops where entering the namespace needs a capability, or at the
    /// namespace that does not exist here.
    #[test]
    fn a_real_pasta_accepts_the_arguments() {
        let bin = std::env::var_os("STORM_PASTA_BIN")
            .map(PathBuf::from)
            .or_else(|| Some(PathBuf::from(PASTA)).filter(|p| p.is_file()));
        let Some(bin) = bin else {
            return;
        };
        let mut args = spec(&[5432, 11434]).pasta_args();
        // Never touch the host's real pid directory.
        let pid = args.iter().position(|a| a == "--pid").unwrap();
        let t = tempfile::tempdir().unwrap();
        args[pid + 1] = t.path().join("pasta.pid").display().to_string();
        args.retain(|a| a != "--quiet");
        let out = std::process::Command::new(&bin)
            .args(&args)
            .arg("--foreground")
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(!out.status.success(), "{stderr}");
        for bad in ["unrecognized option", "Usage:", "Invalid", "invalid"] {
            assert!(!stderr.contains(bad), "{stderr}");
        }
        assert!(
            stderr.contains("namespace") || stderr.contains("Operation not permitted"),
            "{stderr}"
        );
    }
}
