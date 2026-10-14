//! Storm and the host firewall (decision 119, Layers 1 and 3).
//!
//! Storm never opens a port itself. The installer asks before adding one
//! LAN-only rule; this module only describes: the ufw profile and firewalld
//! service the package ships, what `storm-server status` says about the rule,
//! and the command `up` prints when the port changes.

use std::net::IpAddr;
use std::path::Path;

use crate::nettrust::{Iface, Os, Platform, Signal, Snapshot, UNTRUSTED_ZONES};

pub const UFW_APP: &str = "storm-server";
pub const UFW_CONF: &str = "/etc/ufw/ufw.conf";
pub const UFW_PROFILE: &str = "etc/ufw/applications.d/storm-server";
pub const FIREWALLD_SERVICE: &str = "storm-server";
/// The package's copy is under /usr/lib; `up` writes a port override here.
pub const FIREWALLD_OVERRIDE: &str = "etc/firewalld/services/storm-server.xml";
pub const WINDOWS_RULE: &str = "Storm Server";
pub const MACOS_FIREWALL: &str = "/usr/libexec/ApplicationFirewall/socketfilterfw";
pub const PACKAGED_PORT: u16 = 8484;

/// `/etc/ufw/applications.d/storm-server`.
pub fn ufw_profile(port: u16) -> String {
    format!(
        "[{UFW_APP}]\n\
         title=Storm server\n\
         description=Storm notes sync server. Allow it from your local network only.\n\
         ports={port}/tcp\n"
    )
}

/// `/usr/lib/firewalld/services/storm-server.xml`.
pub fn firewalld_service(port: u16) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n\
         <service>\n  \
         <short>Storm server</short>\n  \
         <description>Storm notes sync server. Add it to a home or trusted zone only, never public.</description>\n  \
         <port protocol=\"tcp\" port=\"{port}\"/>\n\
         </service>\n"
    )
}

/// Keeps the ufw profile and the firewalld service on `port`, so a rule that
/// names them follows `storm.env` once it is refreshed. Writes only what the
/// package installed, under `root` (`/` outside tests).
pub fn sync_definitions(root: &Path, port: u16) -> std::io::Result<Vec<String>> {
    let mut wrote = Vec::new();
    let ufw = root.join(UFW_PROFILE);
    if ufw.exists() && std::fs::read_to_string(&ufw)? != ufw_profile(port) {
        std::fs::write(&ufw, ufw_profile(port))?;
        wrote.push(ufw.display().to_string());
    }
    let over = root.join(FIREWALLD_OVERRIDE);
    if port == PACKAGED_PORT {
        if over.exists() {
            std::fs::remove_file(&over)?;
            wrote.push(over.display().to_string());
        }
    } else if over.parent().is_some_and(Path::exists)
        && std::fs::read_to_string(&over).ok() != Some(firewalld_service(port))
    {
        std::fs::write(&over, firewalld_service(port))?;
        wrote.push(over.display().to_string());
    }
    Ok(wrote)
}

/// What `storm-server status` says about reaching Storm from the LAN.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Lan {
    Open(String),
    Blocked { by: String, run: String },
    NoFirewall,
    Unknown(String),
}

impl Lan {
    pub fn describe(&self) -> String {
        match self {
            Lan::Open(rule) => format!("open ({rule})"),
            Lan::Blocked { by, run } => format!("blocked by {by} — run: {run}"),
            Lan::NoFirewall => "no host firewall".into(),
            Lan::Unknown(why) => format!("unknown ({why})"),
        }
    }
}

/// The local RFC 1918 and ULA subnets: what a LAN-only rule allows.
pub fn lan_subnets(ifaces: &[Iface]) -> Vec<String> {
    let mut out: Vec<String> = ifaces
        .iter()
        .filter(|i| match i.ip {
            IpAddr::V4(v4) => v4.is_private(),
            IpAddr::V6(v6) => (v6.segments()[0] & 0xfe00) == 0xfc00,
        })
        .map(Iface::network)
        .collect();
    out.sort();
    out.dedup();
    out
}

pub fn ufw_active(os: &dyn Os) -> bool {
    os.read(UFW_CONF).is_some_and(|conf| {
        conf.lines()
            .any(|l| l.trim().eq_ignore_ascii_case("ENABLED=yes"))
    })
}

pub fn firewalld_running(os: &dyn Os) -> bool {
    os.run("firewall-cmd", &["--state"])
        .is_some_and(|s| s.trim() == "running")
}

fn ufw_allow_commands(os: &dyn Os, ifaces: &[Iface], port: u16) -> String {
    let profile = os.read(&format!("/{UFW_PROFILE}")).is_some();
    let mut subnets = lan_subnets(ifaces);
    if subnets.is_empty() {
        subnets.push("<your LAN subnet>".into());
    }
    subnets
        .iter()
        .map(|s| {
            if profile {
                format!("sudo ufw allow from {s} to any app {UFW_APP}")
            } else {
                format!("sudo ufw allow from {s} to any port {port} proto tcp")
            }
        })
        .collect::<Vec<_>>()
        .join(" && ")
}

/// A rule line of `ufw status verbose`: `To  Action  From`, columns separated
/// by runs of spaces.
fn ufw_columns(line: &str) -> Option<(String, String, String)> {
    let cols: Vec<&str> = line
        .split("  ")
        .map(str::trim)
        .filter(|c| !c.is_empty())
        .collect();
    (cols.len() >= 3).then(|| (cols[0].into(), cols[1].into(), cols[2].into()))
}

fn ufw(os: &dyn Os, ifaces: &[Iface], port: u16) -> Lan {
    let Some(text) = os.run("ufw", &["status", "verbose"]) else {
        return Lan::Unknown(
            "ufw is active and its rules need root: run `sudo storm-server status`".into(),
        );
    };
    if text.lines().any(|l| l.trim() == "Status: inactive") {
        return Lan::NoFirewall;
    }
    if text
        .lines()
        .any(|l| l.starts_with("Default:") && l.contains("allow (incoming)"))
    {
        return Lan::Open("ufw allows all incoming".into());
    }
    let mut stale: Option<String> = None;
    for line in text.lines() {
        let Some((to, action, from)) = ufw_columns(line) else {
            continue;
        };
        if !action.starts_with("ALLOW") {
            continue;
        }
        let rule_port = to
            .split(['/', ' '])
            .next()
            .and_then(|p| p.parse::<u16>().ok());
        let app = to.starts_with(UFW_APP) || to.contains(&format!("({UFW_APP}"));
        let ours = match rule_port {
            Some(p) => p == port,
            None => app,
        };
        if ours {
            return Lan::Open(format!("ufw: {to} from {from}"));
        }
        if app && let Some(p) = rule_port {
            stale = Some(p.to_string());
        }
    }
    match stale {
        Some(old) => Lan::Blocked {
            by: format!("ufw (its rule still names port {old})"),
            run: format!("sudo ufw app update {UFW_APP}"),
        },
        None => Lan::Blocked {
            by: "ufw".into(),
            run: ufw_allow_commands(os, ifaces, port),
        },
    }
}

fn firewalld(os: &dyn Os, port: u16) -> Lan {
    let Some(zones) = os.run("firewall-cmd", &["--get-active-zones"]) else {
        return Lan::Unknown(
            "firewalld is running and would not list its zones: run `sudo storm-server status`"
                .into(),
        );
    };
    let zones = crate::nettrust::parse_active_zones(&zones);
    let trusted: Vec<&str> = zones
        .iter()
        .map(|(z, _)| z.as_str())
        .filter(|z| !UNTRUSTED_ZONES.contains(z))
        .collect();
    let service_ports = os.run(
        "firewall-cmd",
        &[&format!("--info-service={FIREWALLD_SERVICE}")],
    );
    let stale = service_ports.as_deref().and_then(|info| {
        let ports = info
            .lines()
            .find_map(|l| l.trim().strip_prefix("ports:"))?
            .trim()
            .to_string();
        (!ports.split_whitespace().any(|p| p == format!("{port}/tcp"))).then_some(ports)
    });
    for zone in &trusted {
        if *zone == "trusted" {
            return Lan::Open("firewalld zone trusted".into());
        }
        let has = |arg: String| {
            os.run("firewall-cmd", &[&format!("--zone={zone}"), &arg])
                .is_some_and(|s| s.trim() == "yes")
        };
        if has(format!("--query-port={port}/tcp")) {
            return Lan::Open(format!("firewalld: port {port}/tcp in zone {zone}"));
        }
        if has(format!("--query-service={FIREWALLD_SERVICE}")) {
            if let Some(old) = stale {
                return Lan::Blocked {
                    by: format!("firewalld (its {FIREWALLD_SERVICE} service still names {old})"),
                    run: "sudo firewall-cmd --reload".into(),
                };
            }
            return Lan::Open(format!(
                "firewalld: service {FIREWALLD_SERVICE} in zone {zone}"
            ));
        }
    }
    let zone = trusted.first().copied().unwrap_or("home");
    let what = if service_ports.is_some() {
        format!("--add-service={FIREWALLD_SERVICE}")
    } else {
        format!("--add-port={port}/tcp")
    };
    Lan::Blocked {
        by: "firewalld".into(),
        run: format!(
            "sudo firewall-cmd --permanent --zone={zone} {what} && sudo firewall-cmd --reload"
        ),
    }
}

/// The Private-only rule `install.ps1` adds, as a person would type it.
pub fn windows_rule_command(program: &str, port: u16) -> String {
    format!(
        "New-NetFirewallRule -DisplayName '{WINDOWS_RULE}' -Direction Inbound -Program \
         '{program}' -Protocol TCP -LocalPort {port} -Profile Private -Action Allow"
    )
}

const WINDOWS_FIREWALL: &str = "$p = (Get-NetFirewallProfile -Profile Private).Enabled; \
    $r = Get-NetFirewallRule -DisplayName 'Storm Server' -ErrorAction SilentlyContinue | Select-Object -First 1; \
    if ($r) { $f = $r | Get-NetFirewallPortFilter; \
    \"$p`t$($r.Enabled)`t$($r.Profile)`t$($f.LocalPort)`t$($r.Action)\" } else { \"$p\" }";

fn windows(os: &dyn Os, port: u16, program: &str) -> Lan {
    let Some(text) = os.run(
        "powershell.exe",
        &[
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            WINDOWS_FIREWALL,
        ],
    ) else {
        return Lan::Unknown("couldn't read Windows Firewall".into());
    };
    let line = text.lines().next().unwrap_or("").trim_end_matches('\r');
    let f: Vec<&str> = line.split('\t').map(str::trim).collect();
    if f.first().is_some_and(|p| p.eq_ignore_ascii_case("False")) {
        return Lan::NoFirewall;
    }
    let admin = " (PowerShell as administrator)";
    if f.len() < 5 {
        return Lan::Blocked {
            by: "Windows Firewall".into(),
            run: format!("{}{admin}", windows_rule_command(program, port)),
        };
    }
    let (enabled, profile, rule_port, action) = (f[1], f[2], f[3], f[4]);
    if !enabled.eq_ignore_ascii_case("True") || !action.eq_ignore_ascii_case("Allow") {
        return Lan::Blocked {
            by: format!("Windows Firewall (rule '{WINDOWS_RULE}' is off)"),
            run: format!("Enable-NetFirewallRule -DisplayName '{WINDOWS_RULE}'{admin}"),
        };
    }
    if rule_port != port.to_string() {
        return Lan::Blocked {
            by: format!("Windows Firewall (rule '{WINDOWS_RULE}' names port {rule_port})"),
            run: format!(
                "Set-NetFirewallRule -DisplayName '{WINDOWS_RULE}' -LocalPort {port}{admin}"
            ),
        };
    }
    if profile.eq_ignore_ascii_case("Private") {
        Lan::Open(format!(
            "rule '{WINDOWS_RULE}', private networks, TCP {port}"
        ))
    } else {
        Lan::Open(format!(
            "rule '{WINDOWS_RULE}' on {profile} networks — make it private-only: \
             Set-NetFirewallRule -DisplayName '{WINDOWS_RULE}' -Profile Private{admin}"
        ))
    }
}

fn macos(os: &dyn Os, program: &str) -> Lan {
    let Some(state) = os.run(MACOS_FIREWALL, &["--getglobalstate"]) else {
        return Lan::Unknown("couldn't read the macOS firewall".into());
    };
    if state.to_ascii_lowercase().contains("disabled") {
        return Lan::NoFirewall;
    }
    if os
        .run(MACOS_FIREWALL, &["--getblockall"])
        .is_some_and(|s| s.to_ascii_lowercase().contains("enabled"))
    {
        return Lan::Blocked {
            by: "the macOS firewall (Block all incoming connections)".into(),
            run: "turn it off in System Settings › Network › Firewall › Options".into(),
        };
    }
    let app = os
        .run(MACOS_FIREWALL, &["--getappblocked", program])
        .unwrap_or_default()
        .to_ascii_lowercase();
    if app.contains("permitted") || app.contains("allow") {
        Lan::Open("the macOS firewall allows storm-server".into())
    } else if app.contains("block") {
        Lan::Blocked {
            by: "the macOS firewall".into(),
            run: format!("sudo {MACOS_FIREWALL} --unblockapp '{program}'"),
        }
    } else {
        Lan::Unknown(format!(
            "the macOS firewall is on and asks to allow storm-server when a device first \
             connects; or run: sudo {MACOS_FIREWALL} --add '{program}'"
        ))
    }
}

pub fn lan(platform: Platform, os: &dyn Os, ifaces: &[Iface], port: u16, program: &str) -> Lan {
    match platform {
        Platform::Linux if ufw_active(os) => ufw(os, ifaces, port),
        Platform::Linux if firewalld_running(os) => firewalld(os, port),
        Platform::Linux => Lan::NoFirewall,
        Platform::Windows => windows(os, port, program),
        Platform::MacOs => macos(os, program),
    }
}

/// Layer 2 as `status` prints it.
pub fn network(snapshot: &Snapshot) -> String {
    if snapshot.signal == Signal::None {
        return "trusted (no OS signal; the Discoverable switch governs)".into();
    }
    let mut seen: Vec<&str> = Vec::new();
    let mut good = Vec::new();
    let mut bad = Vec::new();
    for v in &snapshot.verdicts {
        if v.iface.ip.is_loopback()
            || !crate::discovery::is_lan_scoped(v.iface.ip)
            || seen.contains(&v.iface.name.as_str())
        {
            continue;
        }
        seen.push(&v.iface.name);
        let entry = format!("{} ({})", v.iface.name, v.reason);
        if v.trusted { &mut good } else { &mut bad }.push(entry);
    }
    if !bad.is_empty() {
        format!(
            "untrusted: {} — not advertised there, password pairing refused",
            bad.join(", ")
        )
    } else if good.is_empty() {
        "no local network".into()
    } else {
        format!("trusted: {}", good.join(", "))
    }
}

/// `storm-server status`'s `lan` and `network` lines.
pub fn status_lines(
    platform: Platform,
    os: &dyn Os,
    ifaces: &[Iface],
    port: u16,
    program: &str,
    width: usize,
) -> Vec<String> {
    let snapshot = crate::nettrust::evaluate(platform, os, ifaces);
    vec![
        format!(
            "{:width$}: {}",
            "lan",
            lan(platform, os, ifaces, port, program).describe()
        ),
        format!("{:width$}: {}", "network", network(&snapshot)),
    ]
}

/// The same, for this machine. `program` is the binary that serves: the
/// Windows rule and the macOS firewall are scoped to it.
pub fn status_here(port: u16, program: &str, width: usize) -> Vec<String> {
    status_lines(
        Platform::current(),
        &crate::nettrust::RealOs,
        &crate::nettrust::local_ifaces(),
        port,
        program,
        width,
    )
}

/// What `up` prints when the port changed and a firewall rule may still name
/// the old one. Storm never edits the rule itself.
pub fn port_change_reminder(
    platform: Platform,
    os: &dyn Os,
    old: u16,
    new: u16,
    program: &str,
) -> Vec<String> {
    if old == new {
        return Vec::new();
    }
    let mut run = Vec::new();
    match platform {
        Platform::Linux => {
            if ufw_active(os) {
                run.push(format!("  sudo ufw app update {UFW_APP}"));
            }
            if firewalld_running(os) {
                run.push("  sudo firewall-cmd --reload".into());
            }
        }
        Platform::Windows => run.push(format!(
            "  Set-NetFirewallRule -DisplayName '{WINDOWS_RULE}' -LocalPort {new}   \
             # PowerShell as administrator; or, if there is no rule yet:\n  {}",
            windows_rule_command(program, new)
        )),
        Platform::MacOs => {}
    }
    if run.is_empty() {
        return Vec::new();
    }
    let mut out = vec![format!(
        "The port changed from {old} to {new}. A firewall rule allowing Storm on your \
         local network still names {old}; update it:"
    )];
    out.extend(run);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nettrust::fake::FakeOs;

    fn iface(name: &str, ip: &str, prefix: u8) -> Iface {
        Iface {
            name: name.into(),
            ip: ip.parse().unwrap(),
            prefix,
        }
    }

    fn laptop() -> Vec<Iface> {
        vec![
            iface("lo", "127.0.0.1", 8),
            iface("wlan0", "192.168.1.13", 24),
            iface("wlan0", "fd12:3456:789a:1::5", 64),
            iface("wlan0", "fe80::1", 64),
            iface("tailscale0", "100.80.1.2", 32),
        ]
    }

    const UFW_ON: &str = "ENABLED=yes\nLOGLEVEL=low\n";
    const UFW_HEAD: &str = "Status: active\nLogging: on (low)\n\
        Default: deny (incoming), allow (outgoing), disabled (routed)\n\
        New profiles: skip\n\n\
        To                         Action      From\n\
        --                         ------      ----\n\
        22/tcp                     ALLOW IN    Anywhere\n";

    fn ufw_os(rules: &str) -> FakeOs {
        FakeOs::default()
            .file(UFW_CONF, UFW_ON)
            .file(&format!("/{UFW_PROFILE}"), &ufw_profile(19484))
            .with("ufw status verbose", &format!("{UFW_HEAD}{rules}"))
    }

    fn line(os: &FakeOs, platform: Platform) -> String {
        status_lines(platform, os, &laptop(), 19484, "/opt/storm-server", 7)[0].clone()
    }

    #[test]
    fn ufw_with_storms_rule_is_open() {
        let os = ufw_os("19484/tcp (storm-server)    ALLOW IN    192.168.1.0/24\n");
        assert_eq!(
            line(&os, Platform::Linux),
            "lan    : open (ufw: 19484/tcp (storm-server) from 192.168.1.0/24)"
        );
        let os = ufw_os("storm-server               ALLOW IN    192.168.1.0/24\n");
        assert_eq!(
            line(&os, Platform::Linux),
            "lan    : open (ufw: storm-server from 192.168.1.0/24)"
        );
    }

    #[test]
    fn ufw_without_a_rule_is_blocked_and_says_the_lan_only_command() {
        assert_eq!(
            line(&ufw_os(""), Platform::Linux),
            "lan    : blocked by ufw — run: \
             sudo ufw allow from 192.168.1.0/24 to any app storm-server && \
             sudo ufw allow from fd12:3456:789a:1::/64 to any app storm-server"
        );
    }

    #[test]
    fn ufw_with_a_rule_on_the_old_port_says_to_refresh_it() {
        let os = ufw_os("8484/tcp (storm-server)    ALLOW IN    192.168.1.0/24\n");
        assert_eq!(
            line(&os, Platform::Linux),
            "lan    : blocked by ufw (its rule still names port 8484) — run: \
             sudo ufw app update storm-server"
        );
    }

    #[test]
    fn ufw_without_root_says_how_to_read_it() {
        let os = FakeOs::default().file(UFW_CONF, UFW_ON);
        assert_eq!(
            line(&os, Platform::Linux),
            "lan    : unknown (ufw is active and its rules need root: run `sudo storm-server status`)"
        );
    }

    #[test]
    fn ufw_allowing_everything_is_open() {
        let os = FakeOs::default().file(UFW_CONF, UFW_ON).with(
            "ufw status verbose",
            "Status: active\nDefault: allow (incoming), allow (outgoing)\n",
        );
        assert_eq!(
            line(&os, Platform::Linux),
            "lan    : open (ufw allows all incoming)"
        );
    }

    fn firewalld_os() -> FakeOs {
        FakeOs::default()
            .with("firewall-cmd --state", "running\n")
            .with(
                "firewall-cmd --get-active-zones",
                "home\n  interfaces: wlan0\n",
            )
            .with(
                "firewall-cmd --info-service=storm-server",
                "storm-server\n  ports: 19484/tcp\n  protocols:\n",
            )
    }

    #[test]
    fn firewalld_with_the_service_in_a_home_zone_is_open() {
        let os = firewalld_os().with(
            "firewall-cmd --zone=home --query-service=storm-server",
            "yes\n",
        );
        assert_eq!(
            line(&os, Platform::Linux),
            "lan    : open (firewalld: service storm-server in zone home)"
        );
    }

    #[test]
    fn firewalld_without_it_is_blocked_and_names_the_home_zone() {
        assert_eq!(
            line(&firewalld_os(), Platform::Linux),
            "lan    : blocked by firewalld — run: sudo firewall-cmd --permanent --zone=home \
             --add-service=storm-server && sudo firewall-cmd --reload"
        );
        let public = FakeOs::default()
            .with("firewall-cmd --state", "running\n")
            .with(
                "firewall-cmd --get-active-zones",
                "public\n  interfaces: wlan0\n",
            )
            .with(
                "firewall-cmd --zone=public --query-service=storm-server",
                "yes\n",
            )
            .with(
                "firewall-cmd --info-service=storm-server",
                "storm-server\n  ports: 19484/tcp\n",
            );
        assert_eq!(
            line(&public, Platform::Linux),
            "lan    : blocked by firewalld — run: sudo firewall-cmd --permanent --zone=home \
             --add-service=storm-server && sudo firewall-cmd --reload",
            "the public zone never counts"
        );
    }

    #[test]
    fn firewalld_with_a_stale_service_port_says_reload() {
        let os = FakeOs::default()
            .with("firewall-cmd --state", "running\n")
            .with(
                "firewall-cmd --get-active-zones",
                "home\n  interfaces: wlan0\n",
            )
            .with(
                "firewall-cmd --info-service=storm-server",
                "storm-server\n  ports: 8484/tcp\n",
            )
            .with(
                "firewall-cmd --zone=home --query-service=storm-server",
                "yes\n",
            );
        assert_eq!(
            line(&os, Platform::Linux),
            "lan    : blocked by firewalld (its storm-server service still names 8484/tcp) — \
             run: sudo firewall-cmd --reload"
        );
    }

    #[test]
    fn no_firewall_on_linux() {
        assert_eq!(
            line(&FakeOs::default(), Platform::Linux),
            "lan    : no host firewall"
        );
        let off = FakeOs::default()
            .file(UFW_CONF, "ENABLED=no\n")
            .with("firewall-cmd --state", "not running\n");
        assert_eq!(line(&off, Platform::Linux), "lan    : no host firewall");
    }

    fn windows_os(answer: &str) -> FakeOs {
        FakeOs::default().with(
            &format!("powershell.exe -NoProfile -NonInteractive -Command {WINDOWS_FIREWALL}"),
            answer,
        )
    }

    #[test]
    fn windows_cases() {
        assert_eq!(
            line(
                &windows_os("True\tTrue\tPrivate\t19484\tAllow\r\n"),
                Platform::Windows
            ),
            "lan    : open (rule 'Storm Server', private networks, TCP 19484)"
        );
        assert_eq!(
            line(&windows_os("True\r\n"), Platform::Windows),
            "lan    : blocked by Windows Firewall — run: New-NetFirewallRule -DisplayName \
             'Storm Server' -Direction Inbound -Program '/opt/storm-server' -Protocol TCP \
             -LocalPort 19484 -Profile Private -Action Allow (PowerShell as administrator)"
        );
        assert_eq!(
            line(
                &windows_os("True\tTrue\tPrivate\t8484\tAllow\r\n"),
                Platform::Windows
            ),
            "lan    : blocked by Windows Firewall (rule 'Storm Server' names port 8484) — run: \
             Set-NetFirewallRule -DisplayName 'Storm Server' -LocalPort 19484 \
             (PowerShell as administrator)"
        );
        assert!(
            line(
                &windows_os("True\tTrue\tAny\t19484\tAllow\r\n"),
                Platform::Windows
            )
            .contains("make it private-only")
        );
        assert_eq!(
            line(&windows_os("False\r\n"), Platform::Windows),
            "lan    : no host firewall"
        );
    }

    #[test]
    fn macos_cases() {
        let fw = |state: &str, app: &str| {
            FakeOs::default()
                .with(&format!("{MACOS_FIREWALL} --getglobalstate"), state)
                .with(
                    &format!("{MACOS_FIREWALL} --getblockall"),
                    "Firewall has block all state set to disabled.\n",
                )
                .with(
                    &format!("{MACOS_FIREWALL} --getappblocked /opt/storm-server"),
                    app,
                )
        };
        assert_eq!(
            line(
                &fw("Firewall is disabled. (State = 0)\n", ""),
                Platform::MacOs
            ),
            "lan    : no host firewall"
        );
        assert_eq!(
            line(
                &fw(
                    "Firewall is enabled. (State = 1)\n",
                    "The application /opt/storm-server is permitted\n"
                ),
                Platform::MacOs
            ),
            "lan    : open (the macOS firewall allows storm-server)"
        );
        assert_eq!(
            line(
                &fw(
                    "Firewall is enabled. (State = 1)\n",
                    "The application /opt/storm-server is blocked\n"
                ),
                Platform::MacOs
            ),
            format!(
                "lan    : blocked by the macOS firewall — run: sudo {MACOS_FIREWALL} \
                 --unblockapp '/opt/storm-server'"
            )
        );
    }

    #[test]
    fn the_network_line_for_each_signal() {
        let lines = |os: &FakeOs| {
            status_lines(Platform::Linux, os, &laptop(), 19484, "storm-server", 7)[1].clone()
        };
        assert_eq!(
            lines(&FakeOs::default()),
            "network: trusted (no OS signal; the Discoverable switch governs)"
        );
        let home = firewalld_os();
        assert_eq!(
            lines(&home),
            "network: trusted: wlan0 (firewalld zone home)"
        );
        let public = FakeOs::default()
            .with("firewall-cmd --state", "running\n")
            .with(
                "firewall-cmd --get-active-zones",
                "public\n  interfaces: wlan0\n",
            );
        assert_eq!(
            lines(&public),
            "network: untrusted: wlan0 (firewalld zone public) — not advertised there, \
             password pairing refused"
        );
    }

    #[test]
    fn a_port_change_reminds_and_prints_the_command() {
        let both = FakeOs::default()
            .file(UFW_CONF, UFW_ON)
            .with("firewall-cmd --state", "running\n");
        assert_eq!(
            port_change_reminder(Platform::Linux, &both, 8484, 19484, "storm-server"),
            [
                "The port changed from 8484 to 19484. A firewall rule allowing Storm on your \
                 local network still names 8484; update it:",
                "  sudo ufw app update storm-server",
                "  sudo firewall-cmd --reload",
            ]
        );
        assert!(
            port_change_reminder(Platform::Linux, &both, 8484, 8484, "storm-server").is_empty()
        );
        assert!(
            port_change_reminder(Platform::Linux, &FakeOs::default(), 8484, 1, "x").is_empty(),
            "no firewall, nothing to say"
        );
        let win = port_change_reminder(
            Platform::Windows,
            &FakeOs::default(),
            8484,
            19484,
            r"C:\Storm\storm-serverw.exe",
        );
        assert!(
            win[1].contains("Set-NetFirewallRule -DisplayName 'Storm Server' -LocalPort 19484")
        );
    }

    #[test]
    fn up_keeps_the_definitions_on_the_port() {
        let dir = tempdir::TempDir::new("storm-fw-defs").unwrap();
        let root = dir.path();
        assert!(
            sync_definitions(root, 19484).unwrap().is_empty(),
            "nothing installed"
        );

        std::fs::create_dir_all(root.join("etc/ufw/applications.d")).unwrap();
        std::fs::write(root.join(UFW_PROFILE), ufw_profile(PACKAGED_PORT)).unwrap();
        std::fs::create_dir_all(root.join("etc/firewalld/services")).unwrap();

        assert_eq!(sync_definitions(root, 19484).unwrap().len(), 2);
        assert_eq!(
            std::fs::read_to_string(root.join(UFW_PROFILE)).unwrap(),
            ufw_profile(19484)
        );
        assert_eq!(
            std::fs::read_to_string(root.join(FIREWALLD_OVERRIDE)).unwrap(),
            firewalld_service(19484)
        );
        assert!(
            sync_definitions(root, 19484).unwrap().is_empty(),
            "idempotent"
        );

        sync_definitions(root, PACKAGED_PORT).unwrap();
        assert!(
            !root.join(FIREWALLD_OVERRIDE).exists(),
            "the package's own copy again"
        );
        assert_eq!(
            std::fs::read_to_string(root.join(UFW_PROFILE)).unwrap(),
            ufw_profile(PACKAGED_PORT)
        );
    }

    #[test]
    fn the_shipped_definitions_are_these() {
        assert_eq!(
            include_str!("../../../deploy/firewall/storm-server.ufw"),
            ufw_profile(PACKAGED_PORT)
        );
        assert_eq!(
            include_str!("../../../deploy/firewall/storm-server.xml"),
            firewalld_service(PACKAGED_PORT)
        );
    }

    #[test]
    fn lan_subnets_are_rfc1918_and_ula_only() {
        assert_eq!(
            lan_subnets(&laptop()),
            ["192.168.1.0/24", "fd12:3456:789a:1::/64"]
        );
    }
}
