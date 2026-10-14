//! Whether the OS calls each network trusted (decision 119, Layer 2).
//!
//! On a network the OS marks untrusted, the server does not advertise
//! `_storm._tcp` and refuses password pairing from it. The signal is the OS's
//! own: the Windows network category, the firewalld zone, or the
//! NetworkManager connection zone. macOS has none, so there the
//! "Discoverable on this network" switch alone governs.

use std::io::Read;
use std::net::IpAddr;
use std::sync::RwLock;
use std::time::{Duration, Instant};

/// firewalld's and NetworkManager's zones for networks nobody vouched for.
pub const UNTRUSTED_ZONES: [&str; 4] = ["public", "external", "drop", "block"];

/// How long one OS query may take before its answer is treated as missing.
/// A cold PowerShell loading the NetTCPIP module can take well over 5 s, and a
/// missed answer means "no OS signal": a Public network then reads as trusted.
const QUERY_TIMEOUT: Duration = Duration::from_secs(20);

/// Runs the OS's own tools. Faked in tests, so every signal source is tested
/// on any host.
pub trait Os: Send + Sync {
    /// stdout of a command that exited 0; `None` for anything else.
    fn run(&self, program: &str, args: &[&str]) -> Option<String>;

    fn read(&self, path: &str) -> Option<String> {
        std::fs::read_to_string(path).ok()
    }
}

pub struct RealOs;

impl Os for RealOs {
    fn run(&self, program: &str, args: &[&str]) -> Option<String> {
        run_with_timeout(program, args, QUERY_TIMEOUT)
    }
}

fn run_with_timeout(program: &str, args: &[&str], timeout: Duration) -> Option<String> {
    let mut cmd = std::process::Command::new(program);
    cmd.args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        // storm-serverw has no console; without this every query would flash
        // a PowerShell window on the person's desktop.
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let mut child = cmd.spawn().ok()?;
    let mut stdout = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut out = Vec::new();
        let _ = stdout.read_to_end(&mut out);
        out
    });
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    };
    let out = reader.join().ok()?;
    status
        .success()
        .then(|| String::from_utf8_lossy(&out).into_owned())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Linux,
    MacOs,
    Windows,
}

impl Platform {
    pub fn current() -> Self {
        if cfg!(windows) {
            Self::Windows
        } else if cfg!(target_os = "macos") {
            Self::MacOs
        } else {
            Self::Linux
        }
    }
}

/// Where the verdicts came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Signal {
    WindowsCategory,
    Firewalld,
    NetworkManager,
    /// No OS signal: every network counts as trusted and the switch governs.
    None,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Iface {
    pub name: String,
    pub ip: IpAddr,
    pub prefix: u8,
}

impl Iface {
    pub fn contains(&self, peer: IpAddr) -> bool {
        match (self.ip, peer) {
            (IpAddr::V4(a), IpAddr::V4(b)) => {
                let bits = u32::from(self.prefix.min(32));
                let mask = u32::MAX.checked_shl(32 - bits).unwrap_or(0);
                u32::from(a) & mask == u32::from(b) & mask
            }
            (IpAddr::V6(a), IpAddr::V6(b)) => {
                let bits = u32::from(self.prefix.min(128));
                let mask = u128::MAX.checked_shl(128 - bits).unwrap_or(0);
                u128::from(a) & mask == u128::from(b) & mask
            }
            (IpAddr::V4(_), IpAddr::V6(b)) => b
                .to_ipv4_mapped()
                .is_some_and(|b| self.contains(IpAddr::V4(b))),
            _ => false,
        }
    }

    /// The subnet, as a firewall rule names it: `192.168.1.0/24`.
    pub fn network(&self) -> String {
        let net = match self.ip {
            IpAddr::V4(a) => {
                let bits = u32::from(self.prefix.min(32));
                let mask = u32::MAX.checked_shl(32 - bits).unwrap_or(0);
                IpAddr::V4((u32::from(a) & mask).into())
            }
            IpAddr::V6(a) => {
                let bits = u32::from(self.prefix.min(128));
                let mask = u128::MAX.checked_shl(128 - bits).unwrap_or(0);
                IpAddr::V6((u128::from(a) & mask).into())
            }
        };
        format!("{net}/{}", self.prefix)
    }
}

/// This host's interface addresses, loopback included.
pub fn local_ifaces() -> Vec<Iface> {
    let mut out: Vec<Iface> = if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .map(|i| {
            let prefix = match &i.addr {
                if_addrs::IfAddr::V4(a) => a.prefixlen,
                if_addrs::IfAddr::V6(a) => a.prefixlen,
            };
            Iface {
                ip: i.ip(),
                name: i.name,
                prefix,
            }
        })
        .collect();
    out.sort_by(|a, b| (&a.name, a.ip).cmp(&(&b.name, b.ip)));
    out
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verdict {
    pub iface: Iface,
    pub trusted: bool,
    /// What the OS said, for logs and `storm-server status`.
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    pub signal: Signal,
    pub verdicts: Vec<Verdict>,
}

impl Snapshot {
    pub fn untrusted(&self) -> impl Iterator<Item = &Verdict> {
        self.verdicts.iter().filter(|v| !v.trusted)
    }
}

/// Classifies every interface by the first OS signal that answers.
pub fn evaluate(platform: Platform, os: &dyn Os, ifaces: &[Iface]) -> Snapshot {
    let found = match platform {
        Platform::Windows => windows(os, ifaces),
        Platform::Linux => firewalld(os, ifaces).or_else(|| network_manager(os, ifaces)),
        Platform::MacOs => None,
    };
    found.unwrap_or_else(|| Snapshot {
        signal: Signal::None,
        verdicts: ifaces
            .iter()
            .map(|i| Verdict {
                iface: i.clone(),
                trusted: true,
                reason: "no OS signal".into(),
            })
            .collect(),
    })
}

fn loopback(i: &Iface) -> Option<Verdict> {
    i.ip.is_loopback().then(|| Verdict {
        iface: i.clone(),
        trusted: true,
        reason: "loopback".into(),
    })
}

fn zone_verdict(i: &Iface, source: &str, zone: Option<&str>) -> Verdict {
    loopback(i).unwrap_or_else(|| match zone {
        Some(z) => Verdict {
            iface: i.clone(),
            trusted: !UNTRUSTED_ZONES.contains(&z),
            reason: format!("{source} zone {z}"),
        },
        None => Verdict {
            iface: i.clone(),
            trusted: true,
            reason: format!("{source}: no zone set"),
        },
    })
}

/// `firewall-cmd --get-active-zones`: a zone name on its own line, then
/// indented `interfaces:` and `sources:` lines. Newer versions mark the default
/// with `(default)`.
pub fn parse_active_zones(text: &str) -> Vec<(String, Vec<String>)> {
    let mut zones: Vec<(String, Vec<String>)> = Vec::new();
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        if !line.starts_with(char::is_whitespace) {
            if let Some(name) = line.split_whitespace().next() {
                zones.push((name.to_string(), Vec::new()));
            }
        } else if let Some(list) = line.trim().strip_prefix("interfaces:")
            && let Some(zone) = zones.last_mut()
        {
            zone.1.extend(list.split_whitespace().map(str::to_string));
        }
    }
    zones
}

fn firewalld(os: &dyn Os, ifaces: &[Iface]) -> Option<Snapshot> {
    if os.run("firewall-cmd", &["--state"])?.trim() != "running" {
        return None;
    }
    let zones = parse_active_zones(&os.run("firewall-cmd", &["--get-active-zones"])?);
    let default = os
        .run("firewall-cmd", &["--get-default-zone"])
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let verdicts = ifaces
        .iter()
        .map(|i| {
            let zone = zones
                .iter()
                .find(|(_, members)| members.contains(&i.name))
                .map(|(z, _)| z.as_str())
                .or(default.as_deref());
            zone_verdict(i, "firewalld", zone)
        })
        .collect();
    Some(Snapshot {
        signal: Signal::Firewalld,
        verdicts,
    })
}

fn network_manager(os: &dyn Os, ifaces: &[Iface]) -> Option<Snapshot> {
    let active = os.run(
        "nmcli",
        &["-t", "-f", "UUID,DEVICE", "connection", "show", "--active"],
    )?;
    let mut zones: Vec<(String, Option<String>)> = Vec::new();
    for line in active.lines() {
        let Some((uuid, device)) = line.split_once(':') else {
            continue;
        };
        if uuid.is_empty() || device.is_empty() {
            continue;
        }
        let zone = os
            .run(
                "nmcli",
                &["-g", "connection.zone", "connection", "show", "uuid", uuid],
            )
            .map(|z| z.trim().to_string())
            .filter(|z| !z.is_empty());
        zones.push((device.to_string(), zone));
    }
    let verdicts = ifaces
        .iter()
        .map(|i| {
            let zone = zones
                .iter()
                .find(|(d, _)| *d == i.name)
                .and_then(|(_, z)| z.as_deref());
            zone_verdict(i, "NetworkManager", zone)
        })
        .collect();
    Some(Snapshot {
        signal: Signal::NetworkManager,
        verdicts,
    })
}

/// One line per address: `<ip>\t<category>\t<alias>`.
const WINDOWS_PROFILES: &str = "Get-NetConnectionProfile | ForEach-Object { \
    $c = $_.NetworkCategory; $a = $_.InterfaceAlias; \
    Get-NetIPAddress -InterfaceIndex $_.InterfaceIndex -ErrorAction SilentlyContinue | \
    ForEach-Object { \"$($_.IPAddress)`t$c`t$a\" } }";

fn windows(os: &dyn Os, ifaces: &[Iface]) -> Option<Snapshot> {
    let text = os.run(
        "powershell.exe",
        &[
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            WINDOWS_PROFILES,
        ],
    )?;
    let profiles: Vec<(IpAddr, String, String)> = text
        .lines()
        .filter_map(|l| {
            let mut parts = l.trim_end_matches('\r').split('\t');
            let ip = parts.next()?.split('%').next()?.trim().parse().ok()?;
            let category = parts.next()?.trim().to_string();
            let alias = parts.next().unwrap_or("").trim().to_string();
            Some((ip, category, alias))
        })
        .collect();
    let verdicts = ifaces
        .iter()
        .map(|i| {
            loopback(i).unwrap_or_else(|| {
                match profiles.iter().find(|(ip, _, _)| *ip == i.ip) {
                    Some((_, category, alias)) => Verdict {
                        iface: i.clone(),
                        trusted: !category.eq_ignore_ascii_case("Public"),
                        reason: format!("Windows network category {category} ({alias})"),
                    },
                    // Windows profiles every connected network, and calls one
                    // it cannot identify Public.
                    None => Verdict {
                        iface: i.clone(),
                        trusted: false,
                        reason: "no Windows network profile".into(),
                    },
                }
            })
        })
        .collect();
    Some(Snapshot {
        signal: Signal::WindowsCategory,
        verdicts,
    })
}

/// The live verdicts, shared by the advertiser and password pairing.
#[derive(Default)]
pub struct NetTrust {
    snapshot: RwLock<Option<Snapshot>>,
    /// Test suites only (`--test-untrusted-interface`): these names are
    /// untrusted whatever the OS says.
    forced_untrusted: Vec<String>,
}

impl NetTrust {
    pub fn new(forced_untrusted: Vec<String>) -> Self {
        Self {
            snapshot: RwLock::new(None),
            forced_untrusted,
        }
    }

    /// Classifies `ifaces` now. True when the verdicts changed.
    pub fn refresh(&self, platform: Platform, os: &dyn Os, ifaces: &[Iface]) -> bool {
        let mut next = evaluate(platform, os, ifaces);
        for v in &mut next.verdicts {
            if self.forced_untrusted.contains(&v.iface.name) {
                v.trusted = false;
                v.reason = "marked untrusted (--test-untrusted-interface)".into();
            }
        }
        let mut slot = self.snapshot.write().unwrap_or_else(|e| e.into_inner());
        let changed = slot.as_ref() != Some(&next);
        *slot = Some(next);
        changed
    }

    pub fn snapshot(&self) -> Option<Snapshot> {
        self.snapshot
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Whether to advertise on the interface holding `ip`. An address not yet
    /// classified is not advertised: a network just joined waits for its
    /// verdict rather than being announced on first.
    pub fn advertisable(&self, ip: IpAddr) -> bool {
        let Some(s) = self.snapshot() else {
            return false;
        };
        let mut known = false;
        for v in s.verdicts.iter().filter(|v| v.iface.ip == ip) {
            if !v.trusted {
                return false;
            }
            known = true;
        }
        known
    }

    /// Whether `peer` is on a network the OS calls untrusted: inside the
    /// subnet of any untrusted interface. A peer on the same subnet as both a
    /// trusted and an untrusted interface is refused; the listener cannot
    /// tell which one it arrived on.
    pub fn peer_untrusted(&self, peer: IpAddr) -> Option<Verdict> {
        let s = self.snapshot()?;
        s.untrusted().find(|v| v.iface.contains(peer)).cloned()
    }
}

#[cfg(test)]
pub mod fake {
    use super::Os;
    use std::collections::HashMap;
    use std::sync::Mutex;

    /// Answers commands from a table; anything else fails, like a missing tool.
    #[derive(Default)]
    pub struct FakeOs {
        pub answers: HashMap<String, String>,
        pub files: HashMap<String, String>,
        pub ran: Mutex<Vec<String>>,
    }

    impl FakeOs {
        pub fn with(mut self, cmd: &str, stdout: &str) -> Self {
            self.answers.insert(cmd.to_string(), stdout.to_string());
            self
        }

        pub fn file(mut self, path: &str, text: &str) -> Self {
            self.files.insert(path.to_string(), text.to_string());
            self
        }
    }

    impl Os for FakeOs {
        fn run(&self, program: &str, args: &[&str]) -> Option<String> {
            let line = std::iter::once(program)
                .chain(args.iter().copied())
                .collect::<Vec<_>>()
                .join(" ");
            self.ran.lock().unwrap().push(line.clone());
            self.answers.get(&line).cloned()
        }

        fn read(&self, path: &str) -> Option<String> {
            self.files.get(path).cloned()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fake::FakeOs;
    use super::*;

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
            iface("enp3s0", "10.0.0.5", 24),
        ]
    }

    fn trusted_of(s: &Snapshot) -> Vec<(&str, bool)> {
        s.verdicts
            .iter()
            .map(|v| (v.iface.name.as_str(), v.trusted))
            .collect()
    }

    #[test]
    fn firewalld_public_external_drop_and_block_are_untrusted() {
        for zone in ["public", "external", "drop", "block"] {
            let os = FakeOs::default()
                .with("firewall-cmd --state", "running\n")
                .with(
                    "firewall-cmd --get-active-zones",
                    &format!("home\n  interfaces: enp3s0\n{zone}\n  interfaces: wlan0\n"),
                )
                .with("firewall-cmd --get-default-zone", "public\n");
            let s = evaluate(Platform::Linux, &os, &laptop());
            assert_eq!(s.signal, Signal::Firewalld);
            assert_eq!(
                trusted_of(&s),
                [("lo", true), ("wlan0", false), ("enp3s0", true)],
                "{zone}"
            );
            assert_eq!(s.verdicts[1].reason, format!("firewalld zone {zone}"));
        }
    }

    #[test]
    fn firewalld_puts_an_unlisted_interface_in_the_default_zone() {
        let os = FakeOs::default()
            .with("firewall-cmd --state", "running\n")
            .with(
                "firewall-cmd --get-active-zones",
                "home (default)\n  interfaces: enp3s0\n  sources: 10.9.0.0/16\n",
            )
            .with("firewall-cmd --get-default-zone", "public\n");
        let s = evaluate(Platform::Linux, &os, &laptop());
        assert_eq!(
            trusted_of(&s),
            [("lo", true), ("wlan0", false), ("enp3s0", true)]
        );
        assert_eq!(s.verdicts[2].reason, "firewalld zone home");
    }

    #[test]
    fn network_manager_zones_govern_without_firewalld() {
        let os = FakeOs::default()
            .with("firewall-cmd --state", "not running\n")
            .with(
                "nmcli -t -f UUID,DEVICE connection show --active",
                "1111:wlan0\n2222:enp3s0\n3333:\n",
            )
            .with(
                "nmcli -g connection.zone connection show uuid 1111",
                "public\n",
            )
            .with("nmcli -g connection.zone connection show uuid 2222", "\n");
        let s = evaluate(Platform::Linux, &os, &laptop());
        assert_eq!(s.signal, Signal::NetworkManager);
        assert_eq!(
            trusted_of(&s),
            [("lo", true), ("wlan0", false), ("enp3s0", true)]
        );
        assert_eq!(s.verdicts[1].reason, "NetworkManager zone public");
        assert_eq!(s.verdicts[2].reason, "NetworkManager: no zone set");
    }

    #[test]
    fn neither_means_trusted_and_the_switch_governs() {
        let s = evaluate(Platform::Linux, &FakeOs::default(), &laptop());
        assert_eq!(s.signal, Signal::None);
        assert!(s.verdicts.iter().all(|v| v.trusted));
    }

    #[test]
    fn macos_has_no_os_signal() {
        let os = FakeOs::default().with("firewall-cmd --state", "running\n");
        let s = evaluate(Platform::MacOs, &os, &laptop());
        assert_eq!(s.signal, Signal::None);
        assert!(s.verdicts.iter().all(|v| v.trusted));
    }

    #[test]
    fn windows_public_category_is_untrusted() {
        let os = FakeOs::default().with(
            &format!(
                "powershell.exe -NoProfile -NonInteractive -Command {WINDOWS_PROFILES}"
            ),
            "192.168.1.13\tPublic\tWi-Fi\r\nfe80::1%12\tPublic\tWi-Fi\r\n10.0.0.5\tPrivate\tEthernet\r\n",
        );
        let mut ifaces = laptop();
        ifaces.push(iface("Wi-Fi", "fe80::1", 64));
        ifaces.push(iface("vEthernet", "172.20.0.1", 20));
        let s = evaluate(Platform::Windows, &os, &ifaces);
        assert_eq!(s.signal, Signal::WindowsCategory);
        assert_eq!(
            trusted_of(&s),
            [
                ("lo", true),
                ("wlan0", false),
                ("enp3s0", true),
                ("Wi-Fi", false),
                ("vEthernet", false),
            ]
        );
        assert_eq!(
            s.verdicts[1].reason,
            "Windows network category Public (Wi-Fi)"
        );
    }

    #[test]
    fn a_failed_windows_query_falls_back_to_the_switch() {
        let s = evaluate(Platform::Windows, &FakeOs::default(), &laptop());
        assert_eq!(s.signal, Signal::None);
    }

    #[test]
    fn subnets_and_membership() {
        let w = iface("wlan0", "192.168.1.13", 24);
        assert!(w.contains("192.168.1.200".parse().unwrap()));
        assert!(w.contains("::ffff:192.168.1.7".parse().unwrap()));
        assert!(!w.contains("192.168.2.1".parse().unwrap()));
        assert_eq!(w.network(), "192.168.1.0/24");
        let u = iface("wlan0", "fd12:3456:789a:1::5", 64);
        assert!(u.contains("fd12:3456:789a:1::99".parse().unwrap()));
        assert!(!u.contains("fd12:3456:789a:2::1".parse().unwrap()));
        assert_eq!(u.network(), "fd12:3456:789a:1::/64");
        assert!(iface("x", "10.1.2.3", 0).contains("8.8.8.8".parse().unwrap()));
    }

    #[test]
    fn the_live_verdicts_gate_advertising_and_pairing() {
        let os = FakeOs::default()
            .with("firewall-cmd --state", "running\n")
            .with(
                "firewall-cmd --get-active-zones",
                "public\n  interfaces: wlan0\nhome\n  interfaces: enp3s0\n",
            );
        let t = NetTrust::default();
        assert!(!t.advertisable("192.168.1.13".parse().unwrap()));
        assert!(t.refresh(Platform::Linux, &os, &laptop()));
        assert!(!t.refresh(Platform::Linux, &os, &laptop()), "unchanged");
        assert!(!t.advertisable("192.168.1.13".parse().unwrap()));
        assert!(t.advertisable("10.0.0.5".parse().unwrap()));
        assert!(t.advertisable("127.0.0.1".parse().unwrap()));
        assert!(
            !t.advertisable("192.168.7.7".parse().unwrap()),
            "not classified yet"
        );
        let refused = t.peer_untrusted("192.168.1.40".parse().unwrap()).unwrap();
        assert_eq!(refused.reason, "firewalld zone public");
        assert!(t.peer_untrusted("10.0.0.9".parse().unwrap()).is_none());
        assert!(t.peer_untrusted("127.0.0.1".parse().unwrap()).is_none());
    }

    #[test]
    fn the_test_flag_marks_an_interface_untrusted() {
        let t = NetTrust::new(vec!["lo".into()]);
        t.refresh(Platform::Linux, &FakeOs::default(), &laptop());
        assert!(!t.advertisable("127.0.0.1".parse().unwrap()));
        assert!(t.peer_untrusted("127.0.0.1".parse().unwrap()).is_some());
        assert!(t.advertisable("192.168.1.13".parse().unwrap()));
    }

    #[test]
    fn a_hung_query_times_out() {
        if cfg!(windows) {
            return;
        }
        let started = Instant::now();
        assert_eq!(
            run_with_timeout("sleep", &["5"], Duration::from_millis(200)),
            None
        );
        assert!(started.elapsed() < Duration::from_secs(2));
        assert_eq!(
            run_with_timeout("echo", &["running"], Duration::from_secs(5)).as_deref(),
            Some("running\n")
        );
        assert_eq!(run_with_timeout("false", &[], Duration::from_secs(5)), None);
    }

    /// The real query on a real Windows runner: it parses, and loopback is
    /// trusted. CI's `server (rust, Windows)` job runs it.
    #[cfg(windows)]
    #[test]
    fn the_real_windows_query_parses() {
        let ifaces = local_ifaces();
        // A cold PowerShell on a loaded runner can miss even QUERY_TIMEOUT;
        // the server re-reads on its refresh interval, so this retries too.
        let mut s = evaluate(Platform::Windows, &RealOs, &ifaces);
        for _ in 0..2 {
            if s.signal == Signal::WindowsCategory {
                break;
            }
            s = evaluate(Platform::Windows, &RealOs, &ifaces);
        }
        assert_eq!(s.signal, Signal::WindowsCategory, "{s:?}");
        assert!(
            s.verdicts
                .iter()
                .filter(|v| v.iface.ip.is_loopback())
                .all(|v| v.trusted)
        );
        assert!(
            s.verdicts
                .iter()
                .any(|v| v.reason.starts_with("Windows network category")),
            "{s:?}"
        );
    }
}
