//! LAN discovery (decision 118): advertises `_storm._tcp` so a device on the
//! same network can find this server and pair with the account password.
//!
//! The advertisement is a hint and carries nothing secret. A client pins the
//! advertised key and runs the identity challenge before it sends anything,
//! and `ops::pair_with_password` refuses every request that is relayed or not
//! from a LAN-scoped peer, whatever this module advertises.

use std::net::IpAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use mdns_sd::{IfKind, IfPredicate, ServiceDaemon, ServiceInfo};

use crate::api::Shared;
use crate::nettrust::{Iface, NetTrust, Platform, RealOs};

pub const SERVICE_TYPE: &str = "_storm._tcp.local.";

/// The TXT `v`: a client ignores an entry whose version it does not know.
pub const TXT_VERSION: &str = "1";

/// Re-checks what is advertised this often: an account created by
/// `storm-server passwd` in another process flips `claimed`, and a network
/// joined or left changes which interfaces may be advertised (decision 119).
const RECHECK: Duration = Duration::from_secs(5);

/// The OS's trust verdicts are re-read at least this often, and whenever the
/// interfaces change (decision 119: "at least every 60 s").
const TRUST_RECHECK: Duration = Duration::from_secs(60);

/// The switch: Settings › Server › "Discoverable on this network", and
/// `--no-discovery` / `STORM_DISCOVERY=0`, which wins over the setting.
pub struct Discovery {
    forced_off: bool,
    enabled: AtomicBool,
    changed: tokio::sync::Notify,
    /// Which networks the OS calls trusted (decision 119, Layer 2).
    pub trust: NetTrust,
}

impl Discovery {
    #[cfg(test)]
    pub fn new(forced_off: bool, enabled: bool) -> Self {
        Self::with_trust(forced_off, enabled, NetTrust::default())
    }

    pub fn with_trust(forced_off: bool, enabled: bool, trust: NetTrust) -> Self {
        Self {
            forced_off,
            enabled: AtomicBool::new(enabled),
            changed: tokio::sync::Notify::new(),
            trust,
        }
    }

    /// Advertising, and password pairing allowed.
    pub fn is_on(&self) -> bool {
        !self.forced_off && self.enabled.load(Ordering::Relaxed)
    }

    pub fn forced_off(&self) -> bool {
        self.forced_off
    }

    /// The persisted setting, which the flag may be overriding.
    pub fn setting(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    pub fn set_enabled(&self, on: bool) {
        self.enabled.store(on, Ordering::Relaxed);
        self.changed.notify_one();
    }

    /// Something in the TXT may have changed (the account was created).
    pub fn poke(&self) {
        self.changed.notify_one();
    }
}

/// Whether `STORM_DISCOVERY` turns discovery off.
pub fn env_forces_off(value: Option<&str>) -> bool {
    matches!(
        value.map(|v| v.trim().to_ascii_lowercase()).as_deref(),
        Some("0" | "false" | "off" | "no")
    )
}

/// Loopback, RFC 1918, link-local or IPv6 unique-local; `100.64.0.0/10` is
/// not. The one rule for where the server advertises and for which peers may
/// pair with a password.
pub fn is_lan_scoped(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_loopback() || v4.is_private() || v4.is_link_local(),
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_lan_scoped(IpAddr::V4(v4));
            }
            let s = v6.segments();
            v6.is_loopback() || (s[0] & 0xfe00) == 0xfc00 || (s[0] & 0xffc0) == 0xfe80
        }
    }
}

/// Whether to advertise on an interface holding `iface`, for a server bound to
/// `bind`. A wildcard bind serves every interface, so every LAN-scoped one is
/// advertised; a concrete bind serves only its own address.
pub fn advertises_on(bind: IpAddr, iface: IpAddr) -> bool {
    if !is_lan_scoped(iface) {
        return false;
    }
    bind.is_unspecified() || bind == iface
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Advert {
    pub instance: String,
    pub txt: Vec<(&'static str, String)>,
}

pub fn advert(
    name: &str,
    server_id: &str,
    public_key_b64: &str,
    claimed: bool,
    lan: bool,
) -> Advert {
    let flag = |b: bool| if b { "1" } else { "0" }.to_string();
    Advert {
        instance: instance_name(name),
        txt: vec![
            ("v", TXT_VERSION.to_string()),
            ("sid", server_id.to_string()),
            ("pk", public_key_b64.to_string()),
            ("ver", env!("CARGO_PKG_VERSION").to_string()),
            ("claimed", flag(claimed)),
            ("lan", flag(lan)),
        ],
    }
}

/// A DNS label is at most 63 bytes.
fn instance_name(name: &str) -> String {
    let name = name.trim();
    let mut out = String::new();
    for c in name.chars().filter(|c| !c.is_control()) {
        if out.len() + c.len_utf8() > 63 {
            break;
        }
        out.push(c);
    }
    if out.is_empty() { "Storm".into() } else { out }
}

/// Never the machine's own hostname: Avahi or Bonjour already answers for
/// that, and two responders for one name is a conflict.
fn host_name(server_id: &str) -> String {
    let tail: String = server_id
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .collect::<String>()
        .to_ascii_lowercase();
    let start = tail.len().saturating_sub(10);
    format!("storm-{}.local.", &tail[start..])
}

struct Running {
    daemon: ServiceDaemon,
    fullname: String,
    advert: Advert,
    allowed: Vec<IpAddr>,
}

impl Running {
    fn start(port: u16, host: &str, advert: Advert, allowed: Vec<IpAddr>) -> mdns_sd::Result<Self> {
        let daemon = ServiceDaemon::new()?;
        daemon.disable_interface(IfKind::All)?;
        let only = allowed.clone();
        daemon.enable_interface(IfKind::Predicate(IfPredicate::new(move |i| {
            only.contains(&i.ip())
        })))?;
        let mut this = Self {
            daemon,
            fullname: String::new(),
            advert: advert.clone(),
            allowed,
        };
        this.register(port, host, advert)?;
        Ok(this)
    }

    fn register(&mut self, port: u16, host: &str, advert: Advert) -> mdns_sd::Result<()> {
        let info = ServiceInfo::new(
            SERVICE_TYPE,
            &advert.instance,
            host,
            "",
            port,
            &advert.txt[..],
        )?
        .enable_addr_auto();
        let fullname = info.get_fullname().to_string();
        if !self.fullname.is_empty() && self.fullname != fullname {
            let _ = self.daemon.unregister(&self.fullname);
        }
        self.daemon.register(info)?;
        self.fullname = fullname;
        self.advert = advert;
        Ok(())
    }

    /// Sends the goodbye, so browsers drop the entry now rather than at TTL.
    async fn stop(self) {
        if let Ok(done) = self.daemon.unregister(&self.fullname) {
            let _ = tokio::task::spawn_blocking(move || done.recv_timeout(Duration::from_secs(1)))
                .await;
        }
        let _ = self.daemon.shutdown();
    }
}

/// The addresses to advertise on: LAN-scoped for this bind, and on a network
/// the OS calls trusted.
pub fn allowed_addresses(bind: IpAddr, ifaces: &[Iface], trust: &NetTrust) -> Vec<IpAddr> {
    let mut out: Vec<IpAddr> = ifaces
        .iter()
        .map(|i| i.ip)
        .filter(|ip| advertises_on(bind, *ip) && trust.advertisable(*ip))
        .collect();
    out.sort();
    out.dedup();
    out
}

/// Re-reads the OS's verdicts when the interfaces changed or a minute passed,
/// and logs each network that became untrusted.
async fn refresh_trust(state: &Shared, last: &mut Option<(std::time::Instant, Vec<Iface>)>) {
    let ifaces = crate::nettrust::local_ifaces();
    let due = match last {
        Some((at, seen)) => *seen != ifaces || at.elapsed() >= TRUST_RECHECK,
        None => true,
    };
    if !due {
        return;
    }
    let st = state.clone();
    let probe = ifaces.clone();
    let changed = tokio::task::spawn_blocking(move || {
        st.discovery
            .trust
            .refresh(Platform::current(), &RealOs, &probe)
    })
    .await
    .unwrap_or(false);
    if changed && let Some(s) = state.discovery.trust.snapshot() {
        for v in s.untrusted().filter(|v| is_lan_scoped(v.iface.ip)) {
            tracing::info!(
                interface = %v.iface.name,
                why = %v.reason,
                "untrusted network: not advertised, password pairing refused"
            );
        }
    }
    *last = Some((std::time::Instant::now(), ifaces));
}

/// Advertises while discovery is on, following the switch, `claimed` and the
/// OS's trust verdicts. Never fatal: a host without multicast still serves
/// everything else.
pub async fn run(
    state: Shared,
    bind: IpAddr,
    port: u16,
    mut stop: tokio::sync::oneshot::Receiver<()>,
) {
    let host = host_name(&state.identity.server_id);
    let mut running: Option<Running> = None;
    let mut failed_with: Option<String> = None;
    let mut trust_seen = None;
    loop {
        let wanted = if state.discovery.is_on() {
            refresh_trust(&state, &mut trust_seen).await;
            let allowed = allowed_addresses(
                bind,
                trust_seen.as_ref().map(|(_, i)| &i[..]).unwrap_or(&[]),
                &state.discovery.trust,
            );
            let claimed = state.auth_db.lock().await.has_account().unwrap_or(false);
            let a = advert(
                &state.identity.name,
                &state.identity.server_id,
                &state.identity.public_key_b64(),
                claimed,
                true,
            );
            (!allowed.is_empty()).then_some((a, allowed))
        } else {
            None
        };

        // A network that became untrusted (or trusted) restarts the
        // responder: mdns-sd does not drop an interface it already serves.
        if let (Some(r), Some((_, allowed))) = (running.as_ref(), wanted.as_ref())
            && r.allowed != *allowed
            && let Some(r) = running.take()
        {
            r.stop().await;
        }

        match (running.as_mut(), wanted) {
            (Some(r), Some((w, _))) if r.advert == w => {}
            (Some(r), Some((w, _))) => {
                if let Err(e) = r.register(port, &host, w) {
                    tracing::warn!(error = %e, "could not update the LAN advertisement");
                }
            }
            (Some(_), None) => {
                if let Some(r) = running.take() {
                    r.stop().await;
                    tracing::info!("stopped advertising on the local network");
                }
            }
            (None, Some((w, allowed))) => match Running::start(port, &host, w, allowed.clone()) {
                Ok(r) => {
                    tracing::info!(
                        service = %r.fullname,
                        port,
                        addresses = ?allowed,
                        "advertising on the local network (_storm._tcp)"
                    );
                    failed_with = None;
                    running = Some(r);
                }
                Err(e) => {
                    let e = e.to_string();
                    if failed_with.as_deref() != Some(&e) {
                        tracing::warn!(error = %e, "could not advertise on the local network");
                    }
                    failed_with = Some(e);
                }
            },
            (None, None) => {}
        }

        tokio::select! {
            _ = state.discovery.changed.notified() => {}
            _ = tokio::time::sleep(RECHECK) => {}
            _ = &mut stop => {
                if let Some(r) = running.take() {
                    r.stop().await;
                }
                return;
            }
        }
    }
}

/// `storm-server status`'s identity lines, read from the running server's
/// `GET /v1/server`: the state directory is the service account's alone, and
/// the operator compares this fingerprint with the one a pairing device shows.
pub fn status_identity(port: u16, width: usize) -> Vec<String> {
    match fetch_server_info(port) {
        Some(info) => identity_lines(&info, width),
        None => vec![format!(
            "{:width$}: unknown (server not answering)",
            "identity"
        )],
    }
}

fn identity_lines(info: &serde_json::Value, width: usize) -> Vec<String> {
    let field = |k: &str| info[k].as_str().unwrap_or("").to_string();
    let key = data_encoding::BASE64URL_NOPAD
        .decode(field("public_key").as_bytes())
        .map(|k| crate::auth::identity::fingerprint(&k))
        .unwrap_or_else(|_| "unknown".into());
    vec![
        format!("{:width$}: {}", "name", field("name")),
        format!("{:width$}: {}", "server", field("server_id")),
        format!("{:width$}: {key}", "key"),
    ]
}

fn fetch_server_info(port: u16) -> Option<serde_json::Value> {
    use std::io::{Read, Write};
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let mut stream = std::net::TcpStream::connect_timeout(&addr, Duration::from_secs(2)).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(2))).ok()?;
    stream
        .write_all(b"GET /v1/server HTTP/1.0\r\nHost: 127.0.0.1\r\n\r\n")
        .ok()?;
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).ok()?;
    let raw = String::from_utf8_lossy(&raw);
    let (head, body) = raw.split_once("\r\n\r\n")?;
    if !head.starts_with("HTTP/1.") || !head.split_whitespace().nth(1)?.starts_with('2') {
        return None;
    }
    serde_json::from_str(body).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_prints_the_fingerprint() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let body = serde_json::json!({
            "server_id": "srv_01ARZ3NDEKTSV4RRFFQ69G5FAV",
            "name": "Dewansh's Storm",
            "public_key": "A6EHv_POEL4dcN0Y50vAmWfk1jCbpQ1fHdyGZBJVMbg",
        })
        .to_string();
        let serve = std::thread::spawn(move || {
            use std::io::{Read, Write};
            let (mut s, _) = listener.accept().unwrap();
            let mut buf = [0u8; 512];
            let n = s.read(&mut buf).unwrap();
            assert!(String::from_utf8_lossy(&buf[..n]).starts_with("GET /v1/server "));
            write!(
                s,
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        });
        let lines = status_identity(port, 9);
        serve.join().unwrap();
        assert_eq!(
            lines,
            [
                "name     : Dewansh's Storm",
                "server   : srv_01ARZ3NDEKTSV4RRFFQ69G5FAV",
                // docs/srp-vectors.json, `key_fingerprint.test_key`.
                "key      : 5647 5AA7 5463 474C",
            ]
        );
    }

    #[test]
    fn status_says_when_the_server_is_not_answering() {
        let port = {
            let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap().port()
        };
        assert_eq!(
            status_identity(port, 8),
            ["identity: unknown (server not answering)"]
        );
    }

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn the_txt_carries_every_required_key_and_nothing_secret() {
        let a = advert(
            "Dewansh's Storm",
            "srv_01ARZ3NDEKTSV4RRFFQ69G5FAV",
            "A6EHv_POEL4dcN0Y50vAmWfk1jCbpQ1fHdyGZBJVMbg",
            true,
            true,
        );
        assert_eq!(a.instance, "Dewansh's Storm");
        let keys: Vec<&str> = a.txt.iter().map(|(k, _)| *k).collect();
        assert_eq!(keys, ["v", "sid", "pk", "ver", "claimed", "lan"]);
        let get = |k: &str| a.txt.iter().find(|(key, _)| *key == k).unwrap().1.clone();
        assert_eq!(get("v"), "1");
        assert_eq!(get("sid"), "srv_01ARZ3NDEKTSV4RRFFQ69G5FAV");
        assert_eq!(get("pk").len(), 43);
        assert_eq!(get("ver"), env!("CARGO_PKG_VERSION"));
        assert_eq!(get("claimed"), "1");
        assert_eq!(get("lan"), "1");

        let fresh = advert("x", "srv_a", "pk", false, false);
        let get = |k: &str| {
            fresh
                .txt
                .iter()
                .find(|(key, _)| *key == k)
                .unwrap()
                .1
                .clone()
        };
        assert_eq!(get("claimed"), "0");
        assert_eq!(get("lan"), "0");
    }

    #[test]
    fn a_txt_that_mdns_accepts() {
        let a = advert(
            "Storm",
            "srv_01ARZ3NDEKTSV4RRFFQ69G5FAV",
            &"A".repeat(43),
            true,
            true,
        );
        ServiceInfo::new(
            SERVICE_TYPE,
            &a.instance,
            &host_name("srv_01ARZ3NDEKTSV4RRFFQ69G5FAV"),
            "",
            8484,
            &a.txt[..],
        )
        .unwrap();
    }

    #[test]
    fn the_instance_name_fits_a_dns_label() {
        assert_eq!(instance_name("  "), "Storm");
        let long = "é".repeat(40);
        let name = instance_name(&long);
        assert!(name.len() <= 63, "{}", name.len());
        assert_eq!(name, "é".repeat(31));
    }

    #[test]
    fn the_host_name_is_storms_own() {
        assert_eq!(
            host_name("srv_01ARZ3NDEKTSV4RRFFQ69G5FAV"),
            "storm-ffq69g5fav.local."
        );
    }

    #[test]
    fn lan_scoped_addresses() {
        for lan in [
            "192.168.1.13",
            "10.0.0.2",
            "172.16.4.1",
            "169.254.3.3",
            "127.0.0.1",
            "::1",
            "fd7a:115c:a1e0::1",
            "fe80::1",
            "::ffff:192.168.1.5",
        ] {
            assert!(is_lan_scoped(ip(lan)), "{lan}");
        }
        for public in [
            "8.8.8.8",
            "100.64.0.1",
            "100.101.102.103",
            "100.127.255.254",
            "100.128.0.1",
            "203.0.113.9",
            "2606:4700::1111",
            "::ffff:8.8.8.8",
        ] {
            assert!(!is_lan_scoped(ip(public)), "{public}");
        }
    }

    #[test]
    fn never_advertised_on_a_public_interface() {
        let any = ip("0.0.0.0");
        assert!(advertises_on(any, ip("192.168.1.13")));
        assert!(!advertises_on(any, ip("100.80.1.2")));
        assert!(!advertises_on(ip("100.80.1.2"), ip("100.80.1.2")));
        assert!(!advertises_on(any, ip("81.2.69.160")));
        assert!(!advertises_on(ip("::"), ip("2a00:1450::1")));
        // A public bind is never advertised, even on its own address.
        assert!(!advertises_on(ip("81.2.69.160"), ip("81.2.69.160")));
        // A concrete bind advertises only where it listens.
        assert!(advertises_on(ip("192.168.1.13"), ip("192.168.1.13")));
        assert!(!advertises_on(ip("192.168.1.13"), ip("10.0.0.2")));
        assert!(advertises_on(ip("127.0.0.1"), ip("127.0.0.1")));
    }

    #[test]
    fn an_untrusted_interface_is_not_advertised() {
        let ifaces = [
            Iface {
                name: "lo".into(),
                ip: ip("127.0.0.1"),
                prefix: 8,
            },
            Iface {
                name: "wlan0".into(),
                ip: ip("192.168.1.13"),
                prefix: 24,
            },
            Iface {
                name: "enp3s0".into(),
                ip: ip("10.0.0.5"),
                prefix: 24,
            },
        ];
        let trust = NetTrust::default();
        assert!(
            allowed_addresses(ip("0.0.0.0"), &ifaces, &trust).is_empty(),
            "nothing before the first verdict"
        );
        let os = crate::nettrust::fake::FakeOs::default()
            .with("firewall-cmd --state", "running\n")
            .with(
                "firewall-cmd --get-active-zones",
                "public\n  interfaces: wlan0\nhome\n  interfaces: enp3s0\n",
            );
        trust.refresh(Platform::Linux, &os, &ifaces);
        assert_eq!(
            allowed_addresses(ip("0.0.0.0"), &ifaces, &trust),
            [ip("10.0.0.5"), ip("127.0.0.1")]
        );
        assert!(allowed_addresses(ip("192.168.1.13"), &ifaces, &trust).is_empty());

        // The network becomes trusted: it is advertised on the next pass.
        let os = crate::nettrust::fake::FakeOs::default()
            .with("firewall-cmd --state", "running\n")
            .with(
                "firewall-cmd --get-active-zones",
                "home\n  interfaces: wlan0 enp3s0\n",
            );
        assert!(trust.refresh(Platform::Linux, &os, &ifaces));
        assert_eq!(
            allowed_addresses(ip("0.0.0.0"), &ifaces, &trust),
            [ip("10.0.0.5"), ip("127.0.0.1"), ip("192.168.1.13")]
        );
    }

    #[test]
    fn the_flag_wins_over_the_setting() {
        let d = Discovery::new(false, true);
        assert!(d.is_on());
        d.set_enabled(false);
        assert!(!d.is_on());
        let forced = Discovery::new(true, true);
        assert!(!forced.is_on());
        assert!(forced.setting());
        assert!(env_forces_off(Some("0")));
        assert!(env_forces_off(Some(" off ")));
        assert!(!env_forces_off(Some("1")));
        assert!(!env_forces_off(None));
    }
}
