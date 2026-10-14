//! The connection race behind `mcp-connect` (decision 105b, SRP §2.2 and
//! §8.3): which path reaches the pinned server, the server's own address or
//! one of its relays, and proof on that path that it is the server.
//!
//! **The rules:**
//! 1. The direct address and every self-hosted relay race together. A public
//!    relay is tried only once every one of those has failed or reached its
//!    own deadline (`T_candidate`), so a fast public relay never beats a slow
//!    self-hosted one. Without E2E, whichever path carries traffic reads it:
//!    preferring Storm's relay over the operator's own would be a privacy
//!    regression, not a latency win.
//! 2. A candidate wins only after [`verify_identity`] passes **over its own
//!    path**, so nothing carrying the key goes anywhere unproven. An impostor
//!    on one path does not stop another path from winning.
//! 3. The losers are dropped, which closes their trunks.
//! 4. When nothing wins, the most telling failure is reported:
//!    `identity_failed` (something answered and could not prove the key),
//!    then `server_offline` (a relay is up and the server is not on it, or
//!    the address refused), then the rest (`relay_down`, `unreachable`, …).
//! 5. After a win, `GET /v1/server` over the winner refreshes the relay set,
//!    cached per `server_id`: the flags only seed it (Q11). Only the proven
//!    winner's set for the pinned `server_id` is adopted, and when it carries
//!    `relays_sig` only if that verifies against the pinned key (§4.3).
//! 6. A `ws://` relay off loopback, RFC 1918, link-local and ULA hosts is not
//!    a candidate and is never dialled or cached ([`plaintext_allowed`]).

use std::collections::BTreeMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::time::Duration;

use futures_util::StreamExt;
use futures_util::stream::FuturesUnordered;
use serde::Deserialize;

use crate::identity::{relay_set_message, verify};
use crate::srp::{SrpTrunk, TrunkConfig, plaintext_allowed};
use crate::transport::{
    Connector, DirectTransport, HttpRequest, HttpResponse, IDLE_LIMIT, ServerPin, SrpTransport,
    Transport, TransportError, verify_identity,
};

/// Relays someone other than the server's operator runs: tier 2, raced last
/// (§2.1, decision 114d). Storm's hosted Cloudflare relay is one whatever its
/// registration mode, since Cloudflare terminates its TLS. The app's
/// `kPublicRelayHosts` holds the same list. A relay not on it is the
/// operator's, which is the safe direction to err.
pub const PUBLIC_RELAY_HOSTS: &[&str] = &["storm-relay.dewansh.space"];

/// `T_candidate` (§8.8): one candidate, first byte to proof.
pub const CANDIDATE_DEADLINE: Duration = Duration::from_secs(15);

/// The cache file under [`crate::platform::user_state_dir`].
pub const CACHE_FILE: &str = "mcp-connect-relays.json";

/// Where a path goes, which decides when it may be raced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    Direct,
    SelfHosted,
    Public,
}

/// A verified path: the winner of a race.
pub enum RaceTransport {
    Direct(DirectTransport),
    Srp(SrpTransport),
}

impl Transport for RaceTransport {
    async fn send(&self, req: HttpRequest) -> Result<HttpResponse, TransportError> {
        match self {
            Self::Direct(t) => t.send(req).await,
            Self::Srp(t) => t.send(req).await,
        }
    }

    fn is_closed(&self) -> bool {
        match self {
            Self::Direct(t) => t.is_closed(),
            Self::Srp(t) => t.is_closed(),
        }
    }

    fn is_lost(&self, error: &TransportError) -> bool {
        match self {
            Self::Direct(t) => t.is_lost(error),
            Self::Srp(t) => t.is_lost(error),
        }
    }

    fn is_relayed(&self) -> bool {
        matches!(self, Self::Srp(_))
    }
}

pub struct RaceOptions {
    /// The server's own address (`http://…`), if any.
    pub direct: Option<String>,
    /// The relays given as flags: the seed.
    pub relays: Vec<String>,
    /// The relay-set cache; `None` neither reads nor writes one.
    pub cache: Option<PathBuf>,
    pub public_hosts: Vec<String>,
    pub trunk: TrunkConfig,
    /// [`IDLE_LIMIT`] outside tests.
    pub idle: Duration,
    /// How long one candidate may take to connect and prove itself.
    pub candidate_deadline: Duration,
}

impl RaceOptions {
    pub fn new(direct: Option<String>, relays: Vec<String>) -> Self {
        Self {
            direct,
            relays,
            cache: crate::platform::user_state_dir().map(|d| d.join(CACHE_FILE)),
            public_hosts: PUBLIC_RELAY_HOSTS.iter().map(|h| h.to_string()).collect(),
            trunk: TrunkConfig::default(),
            idle: IDLE_LIMIT,
            candidate_deadline: CANDIDATE_DEADLINE,
        }
    }
}

pub struct RaceConnector {
    pin: ServerPin,
    options: RaceOptions,
}

/// A relay's base URL, as both the flags and `/v1/server` may spell it: no
/// trailing slash and no `/connect/<server_id>`.
pub fn relay_base(url: &str, server_id: &str) -> String {
    let url = url.trim().trim_end_matches('/');
    url.strip_suffix(&format!("/connect/{server_id}"))
        .unwrap_or(url)
        .to_string()
}

/// §4.3: `wss://` anywhere, `ws://` only on a private network.
fn relay_allowed(url: &str) -> bool {
    !url.starts_with("ws://") || plaintext_allowed(&host_of(url))
}

fn host_of(url: &str) -> String {
    url.parse::<tokio_tungstenite::tungstenite::http::Uri>()
        .ok()
        .and_then(|u| u.host().map(|h| h.to_ascii_lowercase()))
        .unwrap_or_default()
}

impl RaceConnector {
    pub fn new(pin: ServerPin, options: RaceOptions) -> anyhow::Result<Self> {
        if let Some(addr) = &options.direct {
            DirectTransport::new(addr)?;
        }
        Ok(Self { pin, options })
    }

    /// Every relay to try: the flags, then the cache, each once, without the
    /// plaintext relays that may not carry a credential.
    pub fn relays(&self) -> Vec<String> {
        let sid = &self.pin.server_id;
        let cached = self
            .options
            .cache
            .as_deref()
            .map(|p| read_cache(p, sid))
            .unwrap_or_default();
        let mut out: Vec<String> = Vec::new();
        for url in self.options.relays.iter().chain(cached.iter()) {
            let base = relay_base(url, sid);
            if base.is_empty() || out.contains(&base) {
                continue;
            }
            if !relay_allowed(&base) {
                tracing::warn!("{base} is plaintext off a private network; not dialling it");
                continue;
            }
            out.push(base);
        }
        out
    }

    fn tier(&self, relay: &str) -> Tier {
        if self.options.public_hosts.contains(&host_of(relay)) {
            Tier::Public
        } else {
            Tier::SelfHosted
        }
    }

    fn direct_attempt(&self, addr: String) -> Attempt<RaceTransport> {
        let pin = self.pin.clone();
        Box::pin(async move {
            // A fresh client per connect, so nothing pooled from before a
            // failure is reused.
            let t = DirectTransport::new(&addr)
                .map_err(|e| TransportError::new("unreachable", format!("{e:#}")))?;
            verify_identity(&t, &pin).await?;
            Ok(RaceTransport::Direct(t))
        })
    }

    fn relay_attempt(&self, relay: String) -> Attempt<RaceTransport> {
        let pin = self.pin.clone();
        let config = self.options.trunk.clone();
        let idle = self.options.idle;
        Box::pin(async move {
            let key = data_encoding::BASE64URL_NOPAD.encode(pin.key.as_bytes());
            let trunk = SrpTrunk::connect_with(&relay, &pin.server_id, &key, config)
                .await
                .map_err(|e| TransportError::new(e.code(), e.to_string()))?;
            let t = SrpTransport::new(trunk, idle);
            verify_identity(&t, &pin).await?;
            Ok(RaceTransport::Srp(t))
        })
    }

    /// Q11: the relays the server is registered with now, read over the
    /// verified winner, replace this server's cached set. Best effort.
    async fn refresh(&self, winner: &RaceTransport) {
        let Some(path) = self.options.cache.as_deref() else {
            return;
        };
        let response = winner
            .send(HttpRequest {
                method: "GET".into(),
                path_and_query: "/v1/server".into(),
                headers: vec![("accept".into(), "application/json".into())],
                body: vec![],
            })
            .await;
        #[derive(Deserialize)]
        struct Advert {
            url: String,
        }
        #[derive(Deserialize)]
        struct Info {
            server_id: String,
            #[serde(default)]
            relays: Vec<Advert>,
            relays_sig: Option<String>,
        }
        let info = match response {
            Ok(r) if r.status == 200 => serde_json::from_slice::<Info>(&r.body).ok(),
            _ => None,
        };
        let Some(info) = info.filter(|i| i.server_id == self.pin.server_id) else {
            tracing::debug!("the relay set was not refreshed");
            return;
        };
        let advertised: Vec<&str> = info.relays.iter().map(|r| r.url.as_str()).collect();
        if let Some(sig) = &info.relays_sig
            && !verify(
                &self.pin.key,
                &relay_set_message(&self.pin.server_id, &advertised),
                sig,
            )
        {
            tracing::warn!("the relay set's signature does not verify; not adopting it");
            return;
        }
        let relays: Vec<String> = advertised
            .iter()
            .map(|url| relay_base(url, &self.pin.server_id))
            .filter(|url| relay_allowed(url))
            .collect();
        if let Err(e) = write_cache(path, &self.pin.server_id, &relays) {
            tracing::warn!("could not cache the relay set at {}: {e:#}", path.display());
        }
    }
}

impl Connector for RaceConnector {
    type Transport = RaceTransport;

    async fn connect(&self) -> Result<RaceTransport, TransportError> {
        let mut first = Vec::new();
        let mut public = Vec::new();
        if let Some(addr) = &self.options.direct {
            first.push(Candidate {
                label: format!("direct {addr}"),
                attempt: self.direct_attempt(addr.clone()),
            });
        }
        for relay in self.relays() {
            let tier = self.tier(&relay);
            let candidate = Candidate {
                label: format!("relay {relay}"),
                attempt: self.relay_attempt(relay),
            };
            match tier {
                Tier::Public => public.push(candidate),
                _ => first.push(candidate),
            }
        }
        let (label, winner) = race(vec![first, public], self.options.candidate_deadline).await?;
        tracing::info!("reached the server by {label}");
        self.refresh(&winner).await;
        Ok(winner)
    }
}

pub type Attempt<T> = Pin<Box<dyn Future<Output = Result<T, TransportError>> + Send>>;

pub struct Candidate<T> {
    pub label: String,
    pub attempt: Attempt<T>,
}

fn rank(code: &str) -> u8 {
    match code {
        "identity_failed" => 3,
        "server_offline" => 2,
        _ => 1,
    }
}

/// Runs `rounds` in order, each one's candidates together, until one
/// succeeds. A round starts only when every candidate of the one before has
/// failed. Unfinished candidates are dropped when one wins.
pub async fn race<T>(
    rounds: Vec<Vec<Candidate<T>>>,
    deadline: Duration,
) -> Result<(String, T), TransportError> {
    let mut failures: Vec<(String, TransportError)> = Vec::new();
    for round in rounds {
        let mut running: FuturesUnordered<_> = round
            .into_iter()
            .map(|c| async move {
                let outcome = match tokio::time::timeout(deadline, c.attempt).await {
                    Ok(outcome) => outcome,
                    Err(_) => Err(TransportError::new(
                        "server_silent",
                        format!("no proof within {}s", deadline.as_secs()),
                    )),
                };
                (c.label, outcome)
            })
            .collect();
        while let Some((label, outcome)) = running.next().await {
            match outcome {
                Ok(t) => return Ok((label, t)),
                Err(e) => {
                    tracing::debug!(storm_error = e.code, "{label} failed: {}", e.detail);
                    failures.push((label, e));
                }
            }
        }
    }
    let Some(best) = failures
        .iter()
        .enumerate()
        .max_by_key(|(i, (_, e))| (rank(e.code), std::cmp::Reverse(*i)))
        .map(|(_, (_, e))| e.code)
    else {
        return Err(TransportError::new(
            "unreachable",
            "no address and no relay to reach the server by",
        ));
    };
    let detail = failures
        .iter()
        .map(|(label, e)| format!("{label}: {e}"))
        .collect::<Vec<_>>()
        .join("; ");
    Err(TransportError::new(best, detail))
}

/// `{ "<server_id>": ["wss://relay…", …] }`.
type Cache = BTreeMap<String, Vec<String>>;

fn load(path: &Path) -> Cache {
    std::fs::read(path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

pub fn read_cache(path: &Path, server_id: &str) -> Vec<String> {
    load(path).remove(server_id).unwrap_or_default()
}

/// Replaces `server_id`'s entry. The server is the only writer of its relay
/// set, so this mirrors it rather than merging. Written beside and renamed,
/// so a reader never sees half a file.
pub fn write_cache(path: &Path, server_id: &str, relays: &[String]) -> anyhow::Result<()> {
    let mut cache = load(path);
    if cache.get(server_id).map(Vec::as_slice) == Some(relays) {
        return Ok(());
    }
    cache.insert(server_id.to_string(), relays.to_vec());
    if let Some(dir) = path.parent() {
        crate::platform::create_private_dir(dir)?;
    }
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    {
        use std::io::Write;
        let mut file = crate::platform::open_private(
            &tmp,
            std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true),
        )?;
        file.write_all(&serde_json::to_vec_pretty(&cache)?)?;
    }
    std::fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use tokio::sync::oneshot;

    fn fails(code: &'static str) -> Attempt<&'static str> {
        Box::pin(async move { Err(TransportError::new(code, "")) })
    }

    fn wins(name: &'static str) -> Attempt<&'static str> {
        Box::pin(async move { Ok(name) })
    }

    fn candidate(label: &str, attempt: Attempt<&'static str>) -> Candidate<&'static str> {
        Candidate {
            label: label.into(),
            attempt,
        }
    }

    /// Records whether it was ever polled: whether that path was tried.
    fn watched(started: Arc<AtomicBool>, name: &'static str) -> Attempt<&'static str> {
        Box::pin(async move {
            started.store(true, Ordering::SeqCst);
            Ok(name)
        })
    }

    const LONG: Duration = Duration::from_secs(60);

    /// Paused time: a race that started the public round after a delay, as
    /// the app's 2 s does, would have it fire during the sleep.
    #[tokio::test(start_paused = true)]
    async fn a_fast_public_relay_never_beats_a_slow_self_hosted_one() {
        let (release, slow) = oneshot::channel::<()>();
        let public_tried = Arc::new(AtomicBool::new(false));
        let race = tokio::spawn(race(
            vec![
                vec![
                    candidate("direct", fails("unreachable")),
                    candidate(
                        "self-hosted",
                        Box::pin(async move {
                            let _ = slow.await;
                            Ok("self-hosted")
                        }),
                    ),
                ],
                vec![candidate("public", watched(public_tried.clone(), "public"))],
            ],
            LONG,
        ));
        tokio::time::sleep(Duration::from_secs(30)).await;
        assert!(!public_tried.load(Ordering::SeqCst), "the public relay ran");
        release.send(()).unwrap();
        let (label, won) = race.await.unwrap().unwrap();
        assert_eq!((label.as_str(), won), ("self-hosted", "self-hosted"));
        assert!(!public_tried.load(Ordering::SeqCst));
    }

    /// §2.2: no round timer. A round-1 candidate that never answers holds
    /// the public round off until its own `T_candidate` runs out.
    #[tokio::test(start_paused = true)]
    async fn the_public_round_waits_out_a_silent_candidates_deadline() {
        let public_tried = Arc::new(AtomicBool::new(false));
        let race = tokio::spawn(race(
            vec![
                vec![candidate(
                    "self-hosted",
                    Box::pin(std::future::pending::<Result<&'static str, TransportError>>()),
                )],
                vec![candidate("public", watched(public_tried.clone(), "public"))],
            ],
            CANDIDATE_DEADLINE,
        ));
        tokio::time::sleep(CANDIDATE_DEADLINE - Duration::from_millis(100)).await;
        assert!(
            !public_tried.load(Ordering::SeqCst),
            "the public round ran early"
        );
        let (label, _) = race.await.unwrap().unwrap();
        assert_eq!(label, "public");
        assert_eq!(CANDIDATE_DEADLINE, Duration::from_secs(15));
    }

    #[test]
    fn a_plaintext_relay_off_a_private_network_is_not_a_candidate() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("relays.json");
        write_cache(
            &path,
            "srv_A",
            &["ws://100.64.0.1:8080".into(), "ws://10.0.0.2:8080".into()],
        )
        .unwrap();
        let pin = ServerPin {
            server_id: "srv_A".into(),
            key: ed25519_dalek::SigningKey::from_bytes(&[7; 32]).verifying_key(),
        };
        let mut options = RaceOptions::new(
            None,
            vec![
                "ws://relay.example.com".into(),
                "ws://192.168.1.5:8080".into(),
                "wss://relay.example.com".into(),
                "ws://100.100.1.2".into(),
            ],
        );
        options.cache = Some(path);
        let c = RaceConnector::new(pin, options).unwrap();
        assert_eq!(
            c.relays(),
            vec![
                "ws://192.168.1.5:8080",
                "wss://relay.example.com",
                "ws://10.0.0.2:8080"
            ]
        );
    }

    #[tokio::test]
    async fn the_public_relay_runs_only_when_everything_else_failed() {
        let (_, won) = race(
            vec![
                vec![
                    candidate("direct", fails("server_offline")),
                    candidate("self-hosted", fails("relay_down")),
                ],
                vec![candidate("public", wins("public"))],
            ],
            LONG,
        )
        .await
        .unwrap();
        assert_eq!(won, "public");
    }

    #[tokio::test]
    async fn an_impostor_on_one_path_does_not_stop_another_from_winning() {
        let (_, won) = race(
            vec![vec![
                candidate("direct", fails("identity_failed")),
                candidate("self-hosted", wins("self-hosted")),
            ]],
            LONG,
        )
        .await
        .unwrap();
        assert_eq!(won, "self-hosted");
    }

    #[tokio::test]
    async fn identity_failed_outranks_server_offline_which_outranks_the_rest() {
        let all = |codes: &[&'static str]| {
            vec![
                codes
                    .iter()
                    .enumerate()
                    .map(|(i, c)| candidate(&format!("c{i}"), fails(c)))
                    .collect::<Vec<_>>(),
            ]
        };
        for (codes, want) in [
            (
                &["relay_down", "identity_failed", "server_offline"][..],
                "identity_failed",
            ),
            (
                &["unreachable", "server_offline", "relay_down"],
                "server_offline",
            ),
            (&["relay_down", "unreachable"], "relay_down"),
        ] {
            let e = race(all(codes), LONG).await.err().unwrap();
            assert_eq!(e.code, want, "{codes:?}");
            for i in 0..codes.len() {
                assert!(e.detail.contains(&format!("c{i}")), "{}", e.detail);
            }
        }
        // An impostor behind a relay outranks a public relay that is down.
        let e = race(
            vec![
                vec![candidate("self-hosted", fails("identity_failed"))],
                vec![candidate("public", fails("relay_down"))],
            ],
            LONG,
        )
        .await
        .err()
        .unwrap();
        assert_eq!(e.code, "identity_failed");
    }

    #[tokio::test]
    async fn the_losers_are_dropped_when_one_wins() {
        struct Closes(Arc<AtomicBool>);
        impl Drop for Closes {
            fn drop(&mut self) {
                self.0.store(true, Ordering::SeqCst);
            }
        }
        let closed = Arc::new(AtomicBool::new(false));
        let guard = Closes(closed.clone());
        let loser: Attempt<&'static str> = Box::pin(async move {
            let _held = guard;
            std::future::pending::<()>().await;
            Ok("never")
        });
        let (_, won) = race(
            vec![vec![
                candidate("slow", loser),
                candidate("fast", wins("fast")),
            ]],
            LONG,
        )
        .await
        .unwrap();
        assert_eq!(won, "fast");
        assert!(closed.load(Ordering::SeqCst), "the loser was not closed");
    }

    #[tokio::test(start_paused = true)]
    async fn a_candidate_that_never_answers_runs_out_of_time() {
        let e = race(
            vec![vec![candidate(
                "hangs",
                Box::pin(std::future::pending::<Result<&'static str, TransportError>>()),
            )]],
            Duration::from_secs(20),
        )
        .await
        .err()
        .unwrap();
        assert_eq!(e.code, "server_silent");
    }

    #[tokio::test]
    async fn nothing_to_race_is_unreachable() {
        let e = race::<()>(vec![vec![], vec![]], LONG).await.err().unwrap();
        assert_eq!(e.code, "unreachable");
    }

    #[test]
    fn a_relay_is_known_by_its_base_url() {
        for url in [
            "wss://relay.example.com",
            "wss://relay.example.com/",
            "wss://relay.example.com/connect/srv_A",
        ] {
            assert_eq!(relay_base(url, "srv_A"), "wss://relay.example.com");
        }
    }

    #[test]
    fn only_a_listed_host_is_public() {
        let pin = ServerPin {
            server_id: "srv_A".into(),
            key: ed25519_dalek::SigningKey::from_bytes(&[7; 32]).verifying_key(),
        };
        let mut options = RaceOptions::new(None, vec![]);
        options.cache = None;
        options.public_hosts = vec!["relay.storm.example".into()];
        let c = RaceConnector::new(pin, options).unwrap();
        assert_eq!(c.tier("wss://relay.storm.example:443"), Tier::Public);
        assert_eq!(c.tier("wss://RELAY.storm.example"), Tier::Public);
        assert_eq!(c.tier("wss://relay.home.example"), Tier::SelfHosted);
        let hosted = RaceConnector::new(
            c.pin.clone(),
            RaceOptions {
                cache: None,
                ..RaceOptions::new(None, vec![])
            },
        )
        .unwrap();
        assert_eq!(
            hosted.tier("wss://storm-relay.dewansh.space"),
            Tier::Public,
            "Storm's hosted relay is raced in tier 2 (114d)"
        );
        assert_eq!(hosted.tier("wss://relay.home.example"), Tier::SelfHosted);
    }

    #[test]
    fn the_cache_is_per_server_and_replaced_not_merged() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested/relays.json");
        write_cache(&path, "srv_A", &["wss://a".into(), "wss://b".into()]).unwrap();
        write_cache(&path, "srv_B", &["wss://c".into()]).unwrap();
        write_cache(&path, "srv_A", &["wss://b".into()]).unwrap();
        assert_eq!(read_cache(&path, "srv_A"), vec!["wss://b".to_string()]);
        assert_eq!(read_cache(&path, "srv_B"), vec!["wss://c".to_string()]);
        assert!(read_cache(&path, "srv_C").is_empty());
        std::fs::write(&path, "not json").unwrap();
        assert!(read_cache(&path, "srv_A").is_empty());
    }

    #[test]
    fn flags_come_first_and_the_cache_adds_to_them() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("relays.json");
        write_cache(
            &path,
            "srv_A",
            &["wss://cached".into(), "wss://flag".into()],
        )
        .unwrap();
        let pin = ServerPin {
            server_id: "srv_A".into(),
            key: ed25519_dalek::SigningKey::from_bytes(&[7; 32]).verifying_key(),
        };
        let mut options = RaceOptions::new(None, vec!["wss://flag/connect/srv_A".into()]);
        options.cache = Some(path);
        let c = RaceConnector::new(pin, options).unwrap();
        assert_eq!(c.relays(), vec!["wss://flag", "wss://cached"]);
    }
}
