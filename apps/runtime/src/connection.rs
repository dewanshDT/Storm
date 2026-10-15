//! The Connection of SRP §8 (vault *Storm Relay Protocol*), as `mcp-connect`
//! runs it (§8.10, decision 116i): one owner of the race and the proven
//! transport, shared by every message.
//!
//! **The rules:**
//! 1. Single flight: at most one race runs, and a caller that needs the
//!    transport while it runs joins it. The race is its own task, so a
//!    caller giving up never cancels it (§8.3 rule 1).
//! 2. Only a positive failure resets: the transport reporting itself lost
//!    ([`Transport::is_lost`]) or a race ending with no proven candidate.
//!    A stream-scoped error is one failed message and the transport stays
//!    (§8.4).
//! 3. A failed race enters backoff: 1 s → 60 s, ×2, ±20 % jitter, one state
//!    for the Connection. A message in backoff fails at once with the last
//!    race's code, and never races (§8.10 rule 3).
//! 4. Reconnecting is *ensure connected*, never a reset: the first demand
//!    after the backoff races once, and every other caller joins it. There is
//!    no timer, so there is never more than one reconnect pending.
//! 5. The backoff clears only after a transport stayed connected for 60 s.

use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use tokio::sync::watch;
use tokio::time::Instant;

use crate::transport::{Connector, Transport, TransportError};

pub const BACKOFF_FIRST: Duration = Duration::from_secs(1);
pub const BACKOFF_MAX: Duration = Duration::from_secs(60);
/// How long a transport must stay connected before the backoff clears.
pub const BACKOFF_CLEARS_AFTER: Duration = Duration::from_secs(60);
/// A race shorter than this shows the status from before it (§8.6).
const NO_FLICKER: Duration = Duration::from_secs(2);

/// §8.6's status, derived from the state alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Idle,
    Online,
    Relayed,
    Connecting,
    ServerOffline,
    Offline,
    Unproven,
    Closed,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Online => "online",
            Self::Relayed => "relayed",
            Self::Connecting => "connecting",
            Self::ServerOffline => "serverOffline",
            Self::Offline => "offline",
            Self::Unproven => "unproven",
            Self::Closed => "closed",
        }
    }

    fn after_failure(code: &str) -> Self {
        match code {
            "identity_failed" => Self::Unproven,
            "server_offline" => Self::ServerOffline,
            _ => Self::Offline,
        }
    }
}

#[derive(Debug, Default)]
struct Backoff {
    failures: u32,
}

impl Backoff {
    fn next(&mut self) -> Duration {
        let base = BACKOFF_FIRST
            .saturating_mul(1 << self.failures.min(6))
            .min(BACKOFF_MAX);
        self.failures = self.failures.saturating_add(1);
        base.mul_f64(jitter())
    }
}

/// 0.8..=1.2, so many clients of one restarted server do not race in step.
fn jitter() -> f64 {
    use rand::Rng;
    let mut b = [0u8; 4];
    rand::rng().fill_bytes(&mut b);
    0.8 + 0.4 * (u32::from_le_bytes(b) as f64 / u32::MAX as f64)
}

type Outcome<T> = Option<Result<Arc<T>, TransportError>>;

enum State<T> {
    Idle,
    Racing {
        id: u64,
        started: Instant,
        done: watch::Receiver<Outcome<T>>,
    },
    Connected {
        transport: Arc<T>,
        since: Instant,
    },
    /// `Unreachable` or `Unproven`: the last race's error, until `retry_at`.
    Failed {
        error: TransportError,
        retry_at: Instant,
    },
    Closed,
}

struct Inner<T> {
    state: State<T>,
    backoff: Backoff,
    races: u64,
    /// The status before the race in flight, shown for its first 2 s.
    before_race: Status,
}

pub struct Connection<C: Connector> {
    connector: Arc<C>,
    inner: Arc<Mutex<Inner<C::Transport>>>,
}

fn status_of<T: Transport>(state: &State<T>, before_race: Status) -> Status {
    match state {
        State::Idle => Status::Idle,
        State::Racing { started, .. } if started.elapsed() < NO_FLICKER => before_race,
        State::Racing { .. } => Status::Connecting,
        State::Connected { transport, .. } if transport.is_relayed() => Status::Relayed,
        State::Connected { .. } => Status::Online,
        State::Failed { error, .. } => Status::after_failure(error.code),
        State::Closed => Status::Closed,
    }
}

impl<C: Connector + 'static> Connection<C> {
    pub fn new(connector: C) -> Self {
        Self {
            connector: Arc::new(connector),
            inner: Arc::new(Mutex::new(Inner {
                state: State::Idle,
                backoff: Backoff::default(),
                races: 0,
                before_race: Status::Idle,
            })),
        }
    }

    pub fn connector(&self) -> &C {
        &self.connector
    }

    fn lock(&self) -> MutexGuard<'_, Inner<C::Transport>> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn status(&self) -> Status {
        let inner = self.lock();
        status_of(&inner.state, inner.before_race)
    }

    /// The proven transport: the cached one, the race in flight, or a new
    /// race when there is neither and the backoff has run out.
    pub async fn transport(&self) -> Result<Arc<C::Transport>, TransportError> {
        let mut done = {
            let mut inner = self.lock();
            match &inner.state {
                State::Connected { transport, .. } if !transport.is_closed() => {
                    return Ok(transport.clone());
                }
                State::Connected { since, .. } => {
                    let since = *since;
                    tracing::info!("the transport was lost; reconnecting");
                    self.lost(&mut inner, since)
                }
                State::Racing { done, .. } => done.clone(),
                State::Failed { error, retry_at } if Instant::now() < *retry_at => {
                    let wait = retry_at.saturating_duration_since(Instant::now());
                    return Err(TransportError::new(
                        error.code,
                        format!(
                            "{} (not retrying for another {:.1}s)",
                            error.detail,
                            wait.as_secs_f64()
                        ),
                    ));
                }
                State::Failed { .. } | State::Idle => self.race(&mut inner),
                State::Closed => {
                    return Err(TransportError::new("closed", "mcp-connect is closing"));
                }
            }
        };
        match done.wait_for(Option::is_some).await {
            Ok(outcome) => outcome.clone().expect("waited for an outcome"),
            Err(_) => Err(TransportError::new(
                "closed",
                "the connection ended during the race",
            )),
        }
    }

    /// A request on `transport` failed with `error`. Only a transport that
    /// says it is lost ends the connection; anything else is that request's
    /// failure alone.
    pub fn failed(&self, transport: &Arc<C::Transport>, error: &TransportError) {
        if !transport.is_lost(error) {
            return;
        }
        let mut inner = self.lock();
        let since = match &inner.state {
            State::Connected {
                transport: t,
                since,
            } if Arc::ptr_eq(t, transport) => *since,
            _ => return,
        };
        tracing::info!(
            storm_error = error.code,
            "the transport was lost; reconnecting"
        );
        self.lost(&mut inner, since);
    }

    /// Ends the Connection: a race in flight is void, and the transport is
    /// dropped, which closes its trunk.
    pub fn close(&self) {
        self.lock().state = State::Closed;
    }

    fn lost(
        &self,
        inner: &mut Inner<C::Transport>,
        since: Instant,
    ) -> watch::Receiver<Outcome<C::Transport>> {
        if since.elapsed() >= BACKOFF_CLEARS_AFTER {
            inner.backoff = Backoff::default();
        }
        self.race(inner)
    }

    fn race(&self, inner: &mut Inner<C::Transport>) -> watch::Receiver<Outcome<C::Transport>> {
        inner.races += 1;
        let id = inner.races;
        inner.before_race = status_of(&inner.state, inner.before_race);
        let (tx, rx) = watch::channel(None);
        inner.state = State::Racing {
            id,
            started: Instant::now(),
            done: rx.clone(),
        };
        let connector = self.connector.clone();
        let shared = self.inner.clone();
        tokio::spawn(async move {
            let outcome = connector.connect().await.map(Arc::new);
            let mut inner = shared.lock().unwrap_or_else(|e| e.into_inner());
            if !matches!(inner.state, State::Racing { id: current, .. } if current == id) {
                return;
            }
            inner.state = match &outcome {
                Ok(t) => {
                    tracing::info!(
                        status = if t.is_relayed() { "relayed" } else { "online" },
                        "connected; the server's identity is verified"
                    );
                    State::Connected {
                        transport: t.clone(),
                        since: Instant::now(),
                    }
                }
                Err(e) => {
                    let delay = inner.backoff.next();
                    tracing::warn!(
                        storm_error = e.code,
                        status = Status::after_failure(e.code).as_str(),
                        "no proven path to the server; next attempt in {:.1}s: {}",
                        delay.as_secs_f64(),
                        e.detail
                    );
                    State::Failed {
                        error: e.clone(),
                        retry_at: Instant::now() + delay,
                    }
                }
            };
            drop(inner);
            let _ = tx.send(Some(outcome));
        });
        rx
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::{HttpRequest, HttpResponse};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use tokio::sync::Notify;

    struct Path {
        relayed: bool,
        closed: AtomicBool,
    }

    impl Transport for Path {
        async fn send(&self, _: HttpRequest) -> Result<HttpResponse, TransportError> {
            unreachable!()
        }
        fn is_closed(&self) -> bool {
            self.closed.load(Ordering::SeqCst)
        }
        fn is_lost(&self, e: &TransportError) -> bool {
            self.is_closed() || e.code == "trunk_lost"
        }
        fn is_relayed(&self) -> bool {
            self.relayed
        }
    }

    /// Fails with `failing` while it is set; a race waits for `gate` while
    /// that is set.
    struct Fake {
        races: AtomicUsize,
        failing: Mutex<Option<&'static str>>,
        gate: Mutex<Option<Arc<Notify>>>,
    }

    impl Connector for Fake {
        type Transport = Path;
        async fn connect(&self) -> Result<Path, TransportError> {
            self.races.fetch_add(1, Ordering::SeqCst);
            let gate = self.gate.lock().unwrap().clone();
            if let Some(gate) = gate {
                gate.notified().await;
            }
            match *self.failing.lock().unwrap() {
                Some(code) => Err(TransportError::new(code, "fake")),
                None => Ok(Path {
                    relayed: true,
                    closed: AtomicBool::new(false),
                }),
            }
        }
    }

    fn connection(failing: Option<&'static str>) -> Arc<Connection<Fake>> {
        Arc::new(Connection::new(Fake {
            races: AtomicUsize::new(0),
            failing: Mutex::new(failing),
            gate: Mutex::default(),
        }))
    }

    fn races(c: &Connection<Fake>) -> usize {
        c.connector().races.load(Ordering::SeqCst)
    }

    #[tokio::test(start_paused = true)]
    async fn the_backoff_doubles_with_jitter_up_to_a_minute() {
        let mut b = Backoff::default();
        for want in [1u64, 2, 4, 8, 16, 32, 60, 60] {
            let got = b.next().as_secs_f64();
            let want = want as f64;
            assert!(
                got >= want * 0.8 - 1e-9 && got <= want * 1.2 + 1e-9,
                "{got} vs {want}"
            );
        }
    }

    #[tokio::test(start_paused = true)]
    async fn concurrent_callers_join_one_race() {
        let c = connection(None);
        let gate = Arc::new(Notify::new());
        *c.connector().gate.lock().unwrap() = Some(gate.clone());
        let callers: Vec<_> = (0..5)
            .map(|_| {
                let c = c.clone();
                tokio::spawn(async move { c.transport().await.map(|_| ()) })
            })
            .collect();
        tokio::time::sleep(Duration::from_millis(10)).await;
        gate.notify_one();
        for caller in callers {
            caller.await.unwrap().unwrap();
        }
        assert_eq!(races(&c), 1);
        assert_eq!(c.status(), Status::Relayed);
    }

    #[tokio::test(start_paused = true)]
    async fn a_caller_giving_up_never_cancels_the_race() {
        let c = connection(None);
        let gate = Arc::new(Notify::new());
        *c.connector().gate.lock().unwrap() = Some(gate.clone());
        let impatient = tokio::spawn({
            let c = c.clone();
            async move { c.transport().await.map(|_| ()) }
        });
        tokio::time::sleep(Duration::from_millis(10)).await;
        impatient.abort();
        let _ = impatient.await;
        *c.connector().gate.lock().unwrap() = None;
        gate.notify_one();
        tokio::time::sleep(Duration::from_millis(10)).await;
        c.transport().await.unwrap();
        assert_eq!(races(&c), 1, "the abandoned race was re-run");
    }

    #[tokio::test(start_paused = true)]
    async fn in_backoff_a_message_fails_at_once_and_one_reconnect_follows() {
        let c = connection(Some("server_offline"));
        assert_eq!(c.transport().await.err().unwrap().code, "server_offline");
        assert_eq!(c.status(), Status::ServerOffline);
        for _ in 0..10 {
            let e = c.transport().await.err().unwrap();
            assert_eq!(e.code, "server_offline", "{e}");
        }
        assert_eq!(races(&c), 1, "a message in backoff raced");

        tokio::time::sleep(Duration::from_millis(1250)).await;
        *c.connector().failing.lock().unwrap() = None;
        let joined: Vec<_> = (0..5)
            .map(|_| {
                let c = c.clone();
                tokio::spawn(async move { c.transport().await.map(|_| ()) })
            })
            .collect();
        for j in joined {
            j.await.unwrap().unwrap();
        }
        assert_eq!(races(&c), 2, "the backoff's end raced more than once");
    }

    #[tokio::test(start_paused = true)]
    async fn a_stream_scoped_error_keeps_the_transport_and_a_loss_re_races() {
        let c = connection(None);
        let t = c.transport().await.unwrap();
        for code in [
            "rate_limited",
            "stream_aborted",
            "server_timeout",
            "client_backlogged",
            "stream_closed",
            "server_silent",
        ] {
            c.failed(&t, &TransportError::new(code, ""));
            assert!(Arc::ptr_eq(&c.transport().await.unwrap(), &t), "{code}");
        }
        assert_eq!(races(&c), 1);
        c.failed(&t, &TransportError::new("trunk_lost", ""));
        let next = c.transport().await.unwrap();
        assert!(!Arc::ptr_eq(&next, &t));
        assert_eq!(races(&c), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn the_backoff_clears_only_after_a_minute_connected() {
        let c = connection(Some("relay_down"));
        for _ in 0..4 {
            let _ = c.transport().await;
            tokio::time::sleep(BACKOFF_MAX.mul_f64(1.3)).await;
        }
        assert_eq!(c.lock().backoff.failures, 4);

        *c.connector().failing.lock().unwrap() = None;
        let t = c.transport().await.unwrap();
        tokio::time::sleep(Duration::from_secs(5)).await;
        *c.connector().failing.lock().unwrap() = Some("relay_down");
        c.failed(&t, &TransportError::new("trunk_lost", ""));
        let _ = c.transport().await;
        assert_eq!(
            c.lock().backoff.failures,
            5,
            "a short connection cleared it"
        );

        tokio::time::sleep(BACKOFF_MAX.mul_f64(1.3)).await;
        *c.connector().failing.lock().unwrap() = None;
        let t = c.transport().await.unwrap();
        tokio::time::sleep(BACKOFF_CLEARS_AFTER).await;
        *c.connector().failing.lock().unwrap() = Some("relay_down");
        t.closed.store(true, Ordering::SeqCst);
        let _ = c.transport().await;
        assert_eq!(c.lock().backoff.failures, 1);
    }

    #[tokio::test(start_paused = true)]
    async fn status_follows_the_state_and_a_short_race_does_not_flicker() {
        let c = connection(Some("identity_failed"));
        assert_eq!(c.status(), Status::Idle);
        let _ = c.transport().await;
        assert_eq!(c.status(), Status::Unproven);
        tokio::time::sleep(Duration::from_secs(2)).await;

        let gate = Arc::new(Notify::new());
        *c.connector().gate.lock().unwrap() = Some(gate.clone());
        *c.connector().failing.lock().unwrap() = Some("unreachable");
        let racing = tokio::spawn({
            let c = c.clone();
            async move { c.transport().await.map(|_| ()) }
        });
        tokio::time::sleep(Duration::from_millis(500)).await;
        assert_eq!(c.status(), Status::Unproven, "flickered");
        tokio::time::sleep(Duration::from_secs(2)).await;
        assert_eq!(c.status(), Status::Connecting);
        gate.notify_one();
        let _ = racing.await;
        assert_eq!(c.status(), Status::Offline);
        c.close();
        assert_eq!(c.status(), Status::Closed);
    }
}
