//! The relay's three pieces of registration state: pubkey bindings, live
//! challenges, and the registrations themselves.
//!
//! All in memory, except TOFU bindings when the operator gives them a file. The
//! relay is **never an authority** (R5) — it holds no user database, no vault
//! data and no authorization decisions — and nothing here contradicts that: a
//! binding is a routing claim, not an identity.
//!
//! **Bindings held only in memory re-open the trust-on-first-use window on
//! every restart**, for every server that has not reconnected since, and a
//! squatter who registers first in that window owns the `server_id` for good.
//! [`Bindings::load`] closes it: with a bindings file, a pair is on disk before
//! the server is told it is registered.

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use rand::Rng;

use crate::auth::PublicKey;
use crate::config::Allowlist;
use crate::proto::{self, ErrorCode};
use crate::trunk::Tx;

/// What the relay knows about one live server trunk.
///
/// The design note calls the identifier `connection_id`; §6 calls the wire
/// field `trunk_id`. The wire name is used here so one grep finds both.
#[derive(Debug, Clone)]
pub struct Registration {
    pub trunk_id: String,
    /// Wall clock, because this is for an operator reading a status line.
    pub connected_at: SystemTime,
    /// Monotonic, because this is a liveness measure and a clock step
    /// backwards must not make a dead trunk look freshly alive.
    pub last_seen: Instant,
    /// Everything a client needs to route onto this trunk. Behind an `Arc`
    /// because a client trunk holds it for its whole life, while
    /// `Registration` itself is cloned out of the table on every lookup.
    pub trunk: Arc<ServerTrunk>,
}

/// A live server trunk's routing state: the socket to write to, the streams
/// multiplexed onto it, and the client trunks bound to it.
///
/// **A client binds to this object, not to the `server_id`.** Streams are
/// scoped per server trunk (§3), so following a supersession to whatever trunk
/// currently answers for the id would carry a `stream_id` across a boundary it
/// is not defined over — the new trunk has never heard of it.
#[derive(Debug)]
pub struct ServerTrunk {
    tx: Tx,
    streams: Mutex<StreamTable>,
    /// `client_trunk_id` → how to reach that client. Kept so a dying trunk can
    /// tell every one of them, including clients that hold no open stream and
    /// would otherwise wait for a timeout on a destination that is gone.
    clients: Mutex<HashMap<String, Tx>>,
}

/// One multiplexed stream.
///
/// `client` is the whole security property in this file. A `stream_id` maps to
/// exactly one client trunk, so a response can only ever be handed to the
/// client that opened the stream — see [`ServerTrunk::client_for`].
#[derive(Debug, Clone)]
struct Stream {
    client_trunk_id: String,
    client: Tx,
    /// `STREAM_OPEN` sent, `STREAM_ACK` not yet seen. This is what the in-flight
    /// cap counts (§5.1).
    acked: bool,
}

#[derive(Debug, Default)]
struct StreamTable {
    open: HashMap<u32, Stream>,
    /// Monotonic, never reused. `u32` rather than `u16` so id-reuse safety
    /// never has to be reasoned about on a long-lived trunk (§3) — so when it
    /// does run out, the trunk refuses new streams instead of wrapping.
    next_id: u32,
}

/// Why an `OPEN_STREAM` was refused. Both map to `rate_limited`: the client
/// hit a bound, and which bound is the relay's business.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenRefused {
    TooManyInFlight,
    IdsExhausted,
}

impl ServerTrunk {
    pub fn new(tx: Tx) -> Self {
        Self {
            tx,
            streams: Mutex::new(StreamTable {
                open: HashMap::new(),
                // Ids start at 1 so a zero `stream_id` — a field that was
                // defaulted rather than set — is never a live stream.
                next_id: 1,
            }),
            clients: Mutex::new(HashMap::new()),
        }
    }

    /// The socket toward the origin server.
    pub fn tx(&self) -> Tx {
        self.tx.clone()
    }

    pub fn attach_client(&self, client_trunk_id: &str, tx: Tx) {
        self.clients
            .lock()
            .expect("clients mutex")
            .insert(client_trunk_id.to_string(), tx);
    }

    pub fn detach_client(&self, client_trunk_id: &str) {
        self.clients
            .lock()
            .expect("clients mutex")
            .remove(client_trunk_id);
    }

    /// Allocates a `stream_id` and records who owns it.
    ///
    /// **Relay-assigned, never client-asserted** (§3): `OPEN_STREAM` carries no
    /// `stream_id` at all, so there is no value here for a client to influence.
    pub fn open_stream(
        &self,
        client_trunk_id: &str,
        client: Tx,
        max_in_flight: usize,
        max_total: usize,
    ) -> Result<u32, OpenRefused> {
        let mut table = self.streams.lock().expect("streams mutex");

        // Global cap: total concurrent streams across all clients on this trunk.
        if table.open.len() >= max_total {
            return Err(OpenRefused::TooManyInFlight);
        }

        // Counted per client trunk, not per server trunk: a cap shared across
        // clients would let one client's un-acked opens deny service to every
        // other client on the same server.
        let in_flight = table
            .open
            .values()
            .filter(|stream| !stream.acked && stream.client_trunk_id == client_trunk_id)
            .count();
        if in_flight >= max_in_flight {
            return Err(OpenRefused::TooManyInFlight);
        }

        let stream_id = table.next_id;
        let Some(next) = stream_id.checked_add(1) else {
            return Err(OpenRefused::IdsExhausted);
        };
        table.next_id = next;
        table.open.insert(
            stream_id,
            Stream {
                client_trunk_id: client_trunk_id.to_string(),
                client,
                acked: false,
            },
        );
        Ok(stream_id)
    }

    /// Marks a stream acknowledged. `false` if it is not open — a `STREAM_ACK`
    /// for a stream that already timed out, or for an id the relay never
    /// issued.
    pub fn ack_stream(&self, stream_id: u32) -> bool {
        let mut table = self.streams.lock().expect("streams mutex");
        match table.open.get_mut(&stream_id) {
            Some(stream) => {
                stream.acked = true;
                true
            }
            None => false,
        }
    }

    pub fn is_awaiting_ack(&self, stream_id: u32) -> bool {
        self.streams
            .lock()
            .expect("streams mutex")
            .open
            .get(&stream_id)
            .is_some_and(|stream| !stream.acked)
    }

    /// The client a server-ward frame for `stream_id` belongs to.
    ///
    /// The one lookup that decides who receives a response. It answers with the
    /// stream's recorded owner and nothing else, which is what makes cross-talk
    /// unrepresentable rather than merely unlikely.
    pub fn client_for(&self, stream_id: u32) -> Option<Tx> {
        self.streams
            .lock()
            .expect("streams mutex")
            .open
            .get(&stream_id)
            .map(|stream| stream.client.clone())
    }

    /// Whether `client_trunk_id` owns `stream_id`.
    ///
    /// A client naming another client's live stream and a client naming a dead
    /// one are **the same answer on purpose**. Distinguishing them would turn
    /// this into an oracle for which ids are currently in use by somebody else.
    pub fn client_owns(&self, stream_id: u32, client_trunk_id: &str) -> bool {
        self.streams
            .lock()
            .expect("streams mutex")
            .open
            .get(&stream_id)
            .is_some_and(|stream| stream.client_trunk_id == client_trunk_id)
    }

    pub fn close_stream(&self, stream_id: u32) {
        self.streams
            .lock()
            .expect("streams mutex")
            .open
            .remove(&stream_id);
    }

    /// Drops every stream a client trunk owned, returning their ids so the
    /// caller can free them on the server side too (§5.4).
    pub fn close_streams_of(&self, client_trunk_id: &str) -> Vec<u32> {
        let mut table = self.streams.lock().expect("streams mutex");
        let doomed: Vec<u32> = table
            .open
            .iter()
            .filter(|(_, stream)| stream.client_trunk_id == client_trunk_id)
            .map(|(id, _)| *id)
            .collect();
        for id in &doomed {
            table.open.remove(id);
        }
        doomed
    }

    /// Tears the trunk down and tells every client bound to it.
    ///
    /// Each open stream gets its own `ERROR{trunk_lost, stream_id}` so a client
    /// can fail exactly the requests that were in flight. A client with no open
    /// stream still gets one trunk-level `ERROR{trunk_lost}`: its destination is
    /// gone, and leaving it holding an apparently healthy trunk would mean it
    /// only finds out when its next request times out.
    ///
    /// Synchronous and best-effort throughout — this runs on a teardown path
    /// where a peer that is already gone is not an error worth handling.
    pub fn shut_down(&self) {
        self.fail_everyone(ErrorCode::TrunkLost);
    }

    /// Starts the supersession drain (§4.2): streams already open get `drain`
    /// to finish, then anything still open gets `ERROR{trunk_superseded}` and
    /// the superseded trunk's socket is closed.
    ///
    /// New client trunks are already bound to the replacement by the time this
    /// runs, because the registration was swapped before it was called. Clients
    /// bound here keep receiving responses until the window closes.
    ///
    /// Closing the socket is part of the drain, not a courtesy. Without it the
    /// old trunk outlives its window. Usually the server has already dropped
    /// it, but a half-open socket would then sit until the heartbeat deadline.
    pub fn start_drain(self: Arc<Self>, drain: std::time::Duration) {
        let trunk = self;
        tokio::spawn(async move {
            tokio::time::sleep(drain).await;
            // `trunk_superseded`, not `trunk_lost`: the server is alive and has
            // a trunk, so a client should reconnect at once rather than treat
            // the server as down.
            trunk.fail_everyone(ErrorCode::TrunkSuperseded);
            trunk.tx.close();
        });
    }

    /// Tells every client on this trunk that it is over, once each, and
    /// forgets them all: an `ERROR` per open stream, and one trunk-level
    /// `ERROR` for a client that held none.
    fn fail_everyone(&self, code: ErrorCode) {
        let streams = std::mem::take(&mut self.streams.lock().expect("streams mutex").open);
        let clients = std::mem::take(&mut *self.clients.lock().expect("clients mutex"));

        let mut told: HashMap<&str, ()> = HashMap::new();
        for (stream_id, stream) in &streams {
            let _ = stream
                .client
                .try_send_json(proto::error_on_stream(code, *stream_id));
            told.insert(stream.client_trunk_id.as_str(), ());
        }
        for (client_trunk_id, tx) in &clients {
            if !told.contains_key(client_trunk_id.as_str()) {
                let _ = tx.try_send_json(proto::error(code));
            }
            tx.close();
        }
    }

    #[cfg(test)]
    pub fn open_stream_count(&self) -> usize {
        self.streams.lock().expect("streams mutex").open.len()
    }
}

/// Whether a successful signature should record a new trust-on-first-use pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Binding {
    /// The allowlist already vouches for this key, or TOFU recorded it before.
    Established,
    /// No binding exists and there is no allowlist. Record it — but only after
    /// the signature verifies (§4.1).
    RecordOnSuccess,
}

#[derive(Default)]
pub struct Bindings {
    /// `server_id` → the pubkey it is bound to.
    recorded: Mutex<HashMap<String, PublicKey>>,
    /// Where bindings survive a restart. `None` is memory only.
    file: Option<PathBuf>,
}

/// Why a first-use binding was not recorded.
#[derive(Debug)]
pub enum RecordError {
    /// Another key already holds the `server_id`, having won the race to
    /// record it. The peer is told `auth_failed`, like any binding refusal.
    Refused,
    /// The pair could not be made durable, so it was not recorded at all.
    /// A relay-side fault, which the spec has no peer-attributable code for.
    Unpersisted(std::io::Error),
}

/// The first line of a bindings file. The format is the allowlist's on
/// purpose: a bindings file an operator has come to trust can be promoted to
/// `--allowlist` unchanged.
const BINDINGS_HEADER: &str = "\
# storm-relay trust-on-first-use bindings: <server_id> <pubkey>, one per line.
# Written by the relay. Same format as --allowlist, so it can become one as is.
# Deleting a line re-opens that server_id to whoever registers first.
";

impl Bindings {
    /// Bindings that persist to `path`, loaded from it if it exists.
    ///
    /// A missing file is an empty set: that is a first start. A file that does
    /// not parse **stops the relay** rather than starting empty, because an
    /// empty set is every `server_id` open to whoever registers first. Parsing
    /// goes through `Allowlist::parse`, so the file gets the same validation,
    /// including refusing a `server_id` listed twice.
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let recorded = match std::fs::read_to_string(path) {
            Ok(text) => Allowlist::parse(&text)
                .map_err(|why| {
                    anyhow::anyhow!("parsing the bindings file at {}: {why}", path.display())
                })?
                .into_entries(),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => HashMap::new(),
            Err(e) => {
                return Err(anyhow::anyhow!(
                    "reading the bindings file at {}: {e}",
                    path.display()
                ));
            }
        };
        Ok(Self {
            recorded: Mutex::new(recorded),
            file: Some(path.to_path_buf()),
        })
    }

    pub fn len(&self) -> usize {
        self.recorded.lock().expect("bindings mutex").len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Decides whether `pubkey` may claim `server_id`, **before a challenge is
    /// issued** (§4.1: there is no reason to spend a nonce on a registration
    /// that cannot succeed).
    ///
    /// The challenge alone binds nothing — it proves the caller holds the
    /// private key for whatever pubkey it just sent, which an attacker
    /// generating a fresh keypair can do perfectly. This is the check that
    /// makes the pubkey mean something for *this* `server_id`.
    ///
    /// Note what a refusal costs a legitimate operator: **a bound key can never
    /// be replaced.** That permanence is exactly what makes squatting a
    /// `server_id` unattractive, and it is also why there is no key rotation in
    /// v1. Known, deliberate limitation (§7) — a rotation needs its own
    /// authenticated flow, plausibly signed by the *old* key to prove
    /// continuity, and adding a "just re-bind" escape hatch here would delete
    /// the property rather than implement rotation.
    pub fn check(
        &self,
        allowlist: Option<&Allowlist>,
        server_id: &str,
        pubkey: PublicKey,
    ) -> Result<Binding, Refused> {
        if let Some(allowlist) = allowlist {
            // The allowlist *is* the binding. No TOFU, even on first sight:
            // an operator who wrote a file listing which keys may register did
            // not ask for unlisted ones to be adopted on arrival.
            return match allowlist.get(server_id) {
                Some(expected) if expected == pubkey => Ok(Binding::Established),
                _ => Err(Refused),
            };
        }

        let recorded = self.recorded.lock().expect("bindings mutex");
        match recorded.get(server_id) {
            Some(&bound) if bound == pubkey => Ok(Binding::Established),
            Some(_) => Err(Refused),
            None => Ok(Binding::RecordOnSuccess),
        }
    }

    /// Records a trust-on-first-use pair after a verified signature.
    ///
    /// Re-reads under the lock rather than trusting the earlier `check`: two
    /// first-time registrations for one `server_id` can both pass `check` and
    /// both sign correctly, and last-writer-wins would let the slower of them
    /// silently take the binding. The first recorder wins, and the loser is
    /// refused here rather than after it has been told it is registered.
    ///
    /// **With a file, the pair is durable before this returns `Ok`**, and a
    /// write that fails un-records it. A binding the relay announced but could
    /// lose on restart is the first-use window this file exists to close, so
    /// the registration fails instead.
    ///
    /// The write happens under the lock, as blocking IO on an async worker.
    /// Deliberate: it runs once per `server_id` for the life of the relay, and
    /// holding the lock is what stops two first registrations writing the file
    /// out of order.
    pub fn record(&self, server_id: &str, pubkey: PublicKey) -> Result<(), RecordError> {
        let mut recorded = self.recorded.lock().expect("bindings mutex");
        if let Some(&bound) = recorded.get(server_id) {
            return if bound == pubkey {
                Ok(())
            } else {
                Err(RecordError::Refused)
            };
        }
        recorded.insert(server_id.to_string(), pubkey);
        if let Some(file) = &self.file
            && let Err(e) = write_bindings(file, &recorded)
        {
            recorded.remove(server_id);
            return Err(RecordError::Unpersisted(e));
        }
        Ok(())
    }

    #[cfg(test)]
    pub fn bound_key(&self, server_id: &str) -> Option<PublicKey> {
        self.recorded
            .lock()
            .expect("bindings mutex")
            .get(server_id)
            .copied()
    }
}

/// Replaces the bindings file with `recorded`, atomically.
///
/// Temp file, fsync, rename over, fsync the directory. A crash at any point
/// leaves either the old file or the new one, never a truncated one. That
/// matters more than usual here: a torn file stops the relay at the next start
/// (see [`Bindings::load`]), and a missing one would quietly re-open every
/// `server_id`. Sorted, so the file diffs cleanly between writes.
fn write_bindings(path: &Path, recorded: &HashMap<String, PublicKey>) -> std::io::Result<()> {
    let mut entries: Vec<_> = recorded.iter().collect();
    entries.sort_by(|a, b| a.0.cmp(b.0));

    let mut text = String::from(BINDINGS_HEADER);
    for (server_id, key) in entries {
        text.push_str(&format!("{server_id} {}\n", key.to_b64()));
    }

    let tmp = path.with_extension("tmp");
    {
        let mut file = std::fs::File::create(&tmp)?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
    }
    std::fs::rename(&tmp, path)?;
    // The rename is only durable once the directory entry is.
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::File::open(dir)?.sync_all()?;
    }
    Ok(())
}

/// A binding refusal. Carries no reason: the caller must turn every refusal
/// into the one fixed `auth_failed` message (§6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Refused;

/// Issued nonces, so each is answerable exactly once and only while fresh.
///
/// Keyed by nonce alone, not by `(server_id, nonce)`: the connection already
/// remembers which `server_id` it challenged, and this table's only job is
/// single-use plus TTL. Storing the pair would put the same fact in two places.
#[derive(Default)]
pub struct Challenges {
    tracked: Mutex<HashMap<String, Entry>>,
}

#[derive(Debug, Clone, Copy)]
struct Entry {
    expires: Instant,
    spent: bool,
}

impl Challenges {
    /// Records a nonce as answerable for `ttl`.
    ///
    /// **`or_insert`, not `insert`: a spent nonce is never resurrected.** The
    /// generator draws 24 random bytes, so a repeat is not a thing that
    /// happens — which is exactly why the guard belongs here. Single-use is
    /// worth having only if it survives the generator going wrong, and
    /// `insert` would re-arm a nonce that had already been answered and let a
    /// captured signature through a second time. The spent record is kept
    /// (rather than removed on use) for the rest of the TTL, which is the whole
    /// window in which a replay could otherwise matter; after that the nonce is
    /// expired on its own terms.
    pub fn issue(&self, nonce: &str, ttl: Duration, now: Instant) {
        let mut tracked = self.tracked.lock().expect("challenges mutex");
        // Prune on write. Without it an unanswered challenge is a permanent
        // allocation, and the relay authenticates nobody at the door — anyone
        // who can open a socket can ask for a nonce.
        tracked.retain(|_, entry| entry.expires > now);
        tracked.entry(nonce.to_string()).or_insert(Entry {
            expires: now + ttl,
            spent: false,
        });
    }

    /// Spends a nonce. `false` for an unknown, already-spent or expired one —
    /// the three are indistinguishable to the caller by design.
    pub fn consume(&self, nonce: &str, now: Instant) -> bool {
        let mut tracked = self.tracked.lock().expect("challenges mutex");
        match tracked.get_mut(nonce) {
            Some(entry) if !entry.spent && entry.expires > now => {
                entry.spent = true;
                true
            }
            _ => false,
        }
    }

    /// Nonces issued and not yet expired, spent or not.
    ///
    /// Public because it is the only way a test can prove a refusal did **not**
    /// issue a nonce, which is the observable consequence of checking the
    /// binding before the challenge (§4.1).
    pub fn tracked_count(&self) -> usize {
        self.tracked.lock().expect("challenges mutex").len()
    }
}

#[derive(Default)]
pub struct Registrations {
    live: Mutex<HashMap<String, Registration>>,
}

impl Registrations {
    /// Installs a registration, returning the trunk it displaced.
    ///
    /// A new `REGISTER_SERVER` for a `server_id` that already has a live trunk
    /// succeeds (§4.2 supersession): anyone completing the challenge already
    /// holds the private key, so this is a reconnect, not a hijack.
    pub fn install(&self, server_id: &str, registration: Registration) -> Option<Registration> {
        self.live
            .lock()
            .expect("registrations mutex")
            .insert(server_id.to_string(), registration)
    }

    /// Removes a registration **only if `trunk_id` still owns it**.
    ///
    /// The guard is the whole point. When a trunk is superseded, the old
    /// connection then notices its socket is gone and unregisters; an
    /// unconditional remove would delete the *new* trunk's entry and take the
    /// server offline on a successful reconnect.
    pub fn release(&self, server_id: &str, trunk_id: &str) {
        let mut live = self.live.lock().expect("registrations mutex");
        if live.get(server_id).is_some_and(|r| r.trunk_id == trunk_id) {
            live.remove(server_id);
        }
    }

    pub fn touch(&self, server_id: &str, trunk_id: &str, now: Instant) {
        let mut live = self.live.lock().expect("registrations mutex");
        if let Some(registration) = live.get_mut(server_id)
            && registration.trunk_id == trunk_id
        {
            registration.last_seen = now;
        }
    }

    pub fn get(&self, server_id: &str) -> Option<Registration> {
        self.live
            .lock()
            .expect("registrations mutex")
            .get(server_id)
            .cloned()
    }

    pub fn len(&self) -> usize {
        self.live.lock().expect("registrations mutex").len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// A fresh trunk id. Random, not a counter: a counter would leak how many
/// servers have ever registered.
pub fn new_trunk_id() -> String {
    let mut bytes = [0u8; 16];
    rand::rng().fill_bytes(&mut bytes);
    format!("trk_{}", data_encoding::BASE64URL_NOPAD.encode(&bytes))
}

/// A fresh nonce: 24 random bytes, base64url-encoded to 32 characters.
///
/// The base64url alphabet is `A–Z a–z 0–9 - _`, which contains neither `:` nor
/// `"`, so a generated nonce satisfies `validate_nonce` by construction. It is
/// still validated on the way out — generation being correct today is not the
/// same as it being correct after an edit.
pub fn new_nonce() -> String {
    let mut bytes = [0u8; 24];
    rand::rng().fill_bytes(&mut bytes);
    data_encoding::BASE64URL_NOPAD.encode(&bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::validate_nonce;

    fn key(seed: u8) -> PublicKey {
        let signing = ed25519_dalek::SigningKey::from_bytes(&[seed; 32]);
        PublicKey::from_b64(
            &data_encoding::BASE64URL_NOPAD.encode(signing.verifying_key().as_bytes()),
        )
        .unwrap()
    }

    #[test]
    fn a_generated_nonce_satisfies_the_validation_rule() {
        for _ in 0..64 {
            let nonce = new_nonce();
            assert_eq!(nonce.len(), 32);
            validate_nonce(&nonce).unwrap();
        }
    }

    #[test]
    fn tofu_records_on_first_sight_and_refuses_a_different_key_after() {
        let bindings = Bindings::default();
        assert_eq!(
            bindings.check(None, "srv_A", key(1)),
            Ok(Binding::RecordOnSuccess)
        );
        bindings.record("srv_A", key(1)).unwrap();

        assert_eq!(
            bindings.check(None, "srv_A", key(1)),
            Ok(Binding::Established)
        );
        assert_eq!(bindings.check(None, "srv_A", key(2)), Err(Refused));
    }

    #[test]
    fn a_concurrent_second_first_registration_loses_to_the_first_recorder() {
        let bindings = Bindings::default();
        // Both passed `check` before either recorded — the TOCTOU window.
        assert!(bindings.check(None, "srv_A", key(1)).is_ok());
        assert!(bindings.check(None, "srv_A", key(2)).is_ok());
        bindings.record("srv_A", key(1)).unwrap();
        assert!(matches!(
            bindings.record("srv_A", key(2)),
            Err(RecordError::Refused)
        ));
        assert_eq!(bindings.bound_key("srv_A"), Some(key(1)));
    }

    /// A fresh directory for one test's bindings file. Unique per call so the
    /// suite can run in parallel with itself.
    fn scratch_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("storm-relay-{}", new_nonce()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_persisted_binding_survives_a_restart() {
        // The whole point: without this, a restart re-opens the first-use
        // window and whoever registers first owns the id for good.
        let path = scratch_dir().join("bindings");
        let bindings = Bindings::load(&path).unwrap();
        assert!(bindings.is_empty(), "a missing file is a first start");
        bindings.record("srv_A", key(1)).unwrap();
        drop(bindings);

        let restarted = Bindings::load(&path).unwrap();
        assert_eq!(
            restarted.check(None, "srv_A", key(1)),
            Ok(Binding::Established)
        );
        assert_eq!(restarted.check(None, "srv_A", key(2)), Err(Refused));
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn a_bindings_file_is_a_valid_allowlist() {
        // So an operator can promote it to --allowlist unchanged.
        let path = scratch_dir().join("bindings");
        let bindings = Bindings::load(&path).unwrap();
        bindings.record("srv_B", key(2)).unwrap();
        bindings.record("srv_A", key(1)).unwrap();

        let text = std::fs::read_to_string(&path).unwrap();
        let allowlist = Allowlist::parse(&text).unwrap();
        assert_eq!(allowlist.get("srv_A"), Some(key(1)));
        assert_eq!(allowlist.get("srv_B"), Some(key(2)));
        // Sorted, so consecutive writes diff cleanly.
        let a = text.find("srv_A").unwrap();
        let b = text.find("srv_B").unwrap();
        assert!(a < b, "{text}");
        assert!(
            !path.with_extension("tmp").exists(),
            "temp file left behind"
        );
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn a_binding_that_cannot_be_written_is_not_recorded() {
        // The relay must never announce a binding a restart could forget. The
        // directory is removed out from under the store, so the write fails.
        let dir = scratch_dir();
        let bindings = Bindings::load(&dir.join("bindings")).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();

        assert!(matches!(
            bindings.record("srv_A", key(1)),
            Err(RecordError::Unpersisted(_))
        ));
        // Un-recorded: the id is still open, not bound in memory alone.
        assert_eq!(bindings.bound_key("srv_A"), None);
        assert_eq!(
            bindings.check(None, "srv_A", key(2)),
            Ok(Binding::RecordOnSuccess)
        );
    }

    #[test]
    fn a_corrupt_bindings_file_stops_the_relay_rather_than_starting_empty() {
        // Starting empty would open every server_id to whoever registers first.
        let path = scratch_dir().join("bindings");
        std::fs::write(&path, "srv_A not-a-key\n").unwrap();
        assert!(Bindings::load(&path).is_err());

        std::fs::write(
            &path,
            format!("srv_A {}\nsrv_A {}\n", key(1).to_b64(), key(2).to_b64()),
        )
        .unwrap();
        let err = Bindings::load(&path).err().unwrap().to_string();
        assert!(err.contains("listed twice"), "{err}");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn an_allowlist_refuses_an_unlisted_id_even_on_first_sight() {
        let allowlist = Allowlist::from_entries([("srv_A".to_string(), key(1))]);
        let bindings = Bindings::default();

        assert_eq!(
            bindings.check(Some(&allowlist), "srv_A", key(1)),
            Ok(Binding::Established)
        );
        assert_eq!(
            bindings.check(Some(&allowlist), "srv_A", key(2)),
            Err(Refused)
        );
        // Never seen before, and TOFU does not apply because a list exists.
        assert_eq!(
            bindings.check(Some(&allowlist), "srv_B", key(3)),
            Err(Refused)
        );
        assert!(bindings.bound_key("srv_B").is_none());
    }

    #[test]
    fn a_nonce_is_spendable_once_and_only_while_fresh() {
        let challenges = Challenges::default();
        let now = Instant::now();
        challenges.issue("nonce-0123456789", Duration::from_secs(30), now);

        assert!(challenges.consume("nonce-0123456789", now));
        // Replay of the very same nonce.
        assert!(!challenges.consume("nonce-0123456789", now));

        challenges.issue("second-0123456789", Duration::from_secs(30), now);
        assert!(!challenges.consume("second-0123456789", now + Duration::from_secs(31)));
        assert!(!challenges.consume("never-issued-01234", now));
    }

    #[test]
    fn re_issuing_a_spent_nonce_does_not_resurrect_it() {
        // Cannot happen with the real generator; the guard exists so that
        // single-use survives the generator being wrong, which is the only
        // circumstance in which single-use has any work to do.
        let challenges = Challenges::default();
        let now = Instant::now();
        challenges.issue("nonce-0123456789", Duration::from_secs(30), now);
        assert!(challenges.consume("nonce-0123456789", now));

        challenges.issue("nonce-0123456789", Duration::from_secs(30), now);
        assert!(!challenges.consume("nonce-0123456789", now));
    }

    #[test]
    fn issuing_prunes_expired_challenges() {
        let challenges = Challenges::default();
        let now = Instant::now();
        challenges.issue("stale-0123456789ab", Duration::from_secs(30), now);
        assert_eq!(challenges.tracked_count(), 1);
        challenges.issue(
            "fresh-0123456789ab",
            Duration::from_secs(30),
            now + Duration::from_secs(31),
        );
        assert_eq!(challenges.tracked_count(), 1);
    }

    #[test]
    fn a_stream_id_maps_to_exactly_one_client_trunk() {
        // The property the whole design rests on, asserted at the table rather
        // than only over a socket: whatever a frame claims, a stream resolves to
        // the client that opened it and to nobody else.
        let trunk = ServerTrunk::new(Tx::detached(4).0);
        let (alice, _alice_rx) = Tx::detached(4);
        let (bob, _bob_rx) = Tx::detached(4);

        let alice_stream = trunk.open_stream("ctk_alice", alice, 8, 1000).unwrap();
        let bob_stream = trunk.open_stream("ctk_bob", bob, 8, 1000).unwrap();
        assert_ne!(alice_stream, bob_stream);

        assert!(trunk.client_owns(alice_stream, "ctk_alice"));
        assert!(!trunk.client_owns(alice_stream, "ctk_bob"));
        // Same answer as an id that was never issued: a client must not be able
        // to tell "someone else's stream" from "no such stream".
        assert!(!trunk.client_owns(9999, "ctk_bob"));
    }

    #[test]
    fn the_in_flight_cap_is_counted_per_client_and_released_by_an_ack() {
        let trunk = ServerTrunk::new(Tx::detached(4).0);
        let (alice, _alice_rx) = Tx::detached(4);
        let (bob, _bob_rx) = Tx::detached(4);

        let first = trunk
            .open_stream("ctk_alice", alice.clone(), 2, 1000)
            .unwrap();
        trunk
            .open_stream("ctk_alice", alice.clone(), 2, 1000)
            .unwrap();
        assert_eq!(
            trunk.open_stream("ctk_alice", alice.clone(), 2, 1000),
            Err(OpenRefused::TooManyInFlight)
        );

        // Bob is unaffected: a shared cap would hand any anonymous client a way
        // to deny service to every other client on the same server.
        assert!(trunk.open_stream("ctk_bob", bob, 2, 1000).is_ok());

        // The cap counts *un-acked* opens, so an ack frees a slot rather than
        // permanently capping the trunk.
        assert!(trunk.ack_stream(first));
        assert!(trunk.open_stream("ctk_alice", alice, 2, 1000).is_ok());
    }

    #[test]
    fn a_closed_stream_id_is_never_reissued() {
        // Ids are monotonic, not recycled. Reuse would mean a late response for
        // a stream that has closed could be delivered to whoever holds the id
        // now — the cross-talk bug, arriving by a different route.
        let trunk = ServerTrunk::new(Tx::detached(4).0);
        let (client, _rx) = Tx::detached(4);

        let first = trunk.open_stream("ctk_a", client.clone(), 8, 1000).unwrap();
        trunk.close_stream(first);
        let second = trunk.open_stream("ctk_a", client, 8, 1000).unwrap();
        assert_ne!(first, second);
        assert_eq!(trunk.open_stream_count(), 1);
    }

    #[test]
    fn a_superseded_trunks_cleanup_does_not_evict_its_replacement() {
        let registrations = Registrations::default();
        let make = |trunk_id: &str| Registration {
            trunk_id: trunk_id.to_string(),
            connected_at: SystemTime::now(),
            last_seen: Instant::now(),
            trunk: Arc::new(ServerTrunk::new(Tx::detached(1).0)),
        };

        registrations.install("srv_A", make("trk_old"));
        let displaced = registrations.install("srv_A", make("trk_new"));
        assert_eq!(displaced.unwrap().trunk_id, "trk_old");

        // The old connection now notices its socket closed and cleans up.
        registrations.release("srv_A", "trk_old");
        assert_eq!(registrations.get("srv_A").unwrap().trunk_id, "trk_new");

        registrations.release("srv_A", "trk_new");
        assert!(registrations.is_empty());
    }
}
