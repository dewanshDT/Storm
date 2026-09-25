//! TLS terminated by the relay itself (decision 71).
//!
//! **Why not a reverse proxy.** The relay takes a client's address from the
//! accepted socket, never from a header, because a header is something the
//! client wrote (§5.2). Behind a TLS proxy every socket is the proxy's, so the
//! per-IP `HELLO` limit would become one bucket for everyone, and the origin's
//! login limiter would see one `relay_peer_ip` for every relayed client.
//! Terminating TLS here keeps the socket, and so the address, the client's own.
//!
//! Two properties are the point of this file:
//!
//! - **A handshake never blocks an accept.** Each one runs in its own task under
//!   a deadline and a concurrency cap. A handshake inside `Listener::accept`
//!   would let one client that opens a socket and says nothing stall every
//!   connection behind it.
//! - **The certificate reloads without a restart.** [`CertStore::reload`] swaps
//!   it in place, driven by `SIGHUP` in `main` (`systemctl reload`). A restart
//!   would drop every trunk and make each server re-register. A reload that
//!   fails keeps the certificate that works.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use rustls_pki_types::pem::PemObject;
use rustls_pki_types::{CertificateDer, PrivateKeyDer};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Semaphore, mpsc};
use tokio_rustls::TlsAcceptor;
use tokio_rustls::rustls::server::{ClientHello, ResolvesServerCert};
use tokio_rustls::rustls::sign::CertifiedKey;
use tokio_rustls::rustls::{self, ServerConfig};
use tokio_rustls::server::TlsStream;

/// How long a client may take to finish a TLS handshake.
///
/// The client has not said anything the relay can act on yet, so this is purely
/// a bound on what an idle socket costs.
pub const DEFAULT_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// Handshakes in progress at once. Past this, a new connection is dropped
/// without a handshake, which bounds the memory and tasks an anonymous caller
/// can hold open by connecting and stalling.
pub const MAX_CONCURRENT_HANDSHAKES: usize = 1024;

/// The certificate and key files, and the pair currently being served.
#[derive(Debug)]
pub struct CertStore {
    cert: PathBuf,
    key: PathBuf,
    current: RwLock<Arc<CertifiedKey>>,
}

impl CertStore {
    /// Loads the pair, refusing a key that does not belong to the certificate.
    /// The relay would otherwise start, and fail every handshake.
    pub fn load(cert: &Path, key: &Path) -> Result<Self> {
        let current = load_pair(cert, key)?;
        Ok(Self {
            cert: cert.to_path_buf(),
            key: key.to_path_buf(),
            current: RwLock::new(Arc::new(current)),
        })
    }

    /// Re-reads both files and swaps the pair in.
    ///
    /// On any error the old pair stays. A renewal hook that wrote half a file,
    /// or a key the certificate does not match, costs a log line, never the
    /// relay. Connections already open keep the session they negotiated.
    pub fn reload(&self) -> Result<()> {
        let fresh = load_pair(&self.cert, &self.key)?;
        *self.current.write().expect("cert store lock") = Arc::new(fresh);
        Ok(())
    }

    /// The server config every handshake uses. ALPN is HTTP/1.1 only: a
    /// WebSocket upgrade is an HTTP/1.1 exchange, and offering `h2` would let a
    /// client negotiate a protocol it cannot upgrade over.
    pub fn server_config(self: &Arc<Self>) -> Result<Arc<ServerConfig>> {
        let mut config =
            ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
                .with_safe_default_protocol_versions()
                .context("configuring TLS protocol versions")?
                .with_no_client_auth()
                .with_cert_resolver(self.clone());
        config.alpn_protocols = vec![b"http/1.1".to_vec()];
        Ok(Arc::new(config))
    }
}

impl ResolvesServerCert for CertStore {
    fn resolve(&self, _hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        Some(self.current.read().expect("cert store lock").clone())
    }
}

fn load_pair(cert: &Path, key: &Path) -> Result<CertifiedKey> {
    let chain = CertificateDer::pem_file_iter(cert)
        .with_context(|| format!("reading the TLS certificate at {}", cert.display()))?
        .collect::<std::result::Result<Vec<_>, _>>()
        .with_context(|| format!("parsing the TLS certificate at {}", cert.display()))?;
    if chain.is_empty() {
        return Err(anyhow!("no certificate in {}", cert.display()));
    }
    let key_der = PrivateKeyDer::from_pem_file(key)
        .with_context(|| format!("reading the TLS key at {}", key.display()))?;
    let signing = rustls::crypto::ring::sign::any_supported_type(&key_der)
        .map_err(|e| anyhow!("unusable TLS key at {}: {e}", key.display()))?;
    let pair = CertifiedKey::new(chain, signing);
    pair.keys_match().map_err(|e| {
        anyhow!(
            "the key at {} does not belong to the certificate at {}: {e}",
            key.display(),
            cert.display()
        )
    })?;
    Ok(pair)
}

/// A listener that hands axum connections whose TLS handshake has already
/// finished, each with the address of the socket it arrived on.
pub struct TlsListener {
    ready: mpsc::Receiver<(TlsStream<TcpStream>, SocketAddr)>,
    local: SocketAddr,
}

impl TlsListener {
    pub fn new(tcp: TcpListener, config: Arc<ServerConfig>, handshake_timeout: Duration) -> Self {
        let local = tcp
            .local_addr()
            .expect("a bound TcpListener has a local address");
        // Small: a completed handshake waits here only until axum's loop takes
        // it, which it does as fast as it can spawn a connection task.
        let (tx, ready) = mpsc::channel(64);
        tokio::spawn(accept_loop(
            tcp,
            TlsAcceptor::from(config),
            handshake_timeout,
            tx,
        ));
        Self { ready, local }
    }
}

async fn accept_loop(
    tcp: TcpListener,
    acceptor: TlsAcceptor,
    handshake_timeout: Duration,
    ready: mpsc::Sender<(TlsStream<TcpStream>, SocketAddr)>,
) {
    let permits = Arc::new(Semaphore::new(MAX_CONCURRENT_HANDSHAKES));
    loop {
        let (socket, peer) = match tcp.accept().await {
            Ok(accepted) => accepted,
            Err(e) => {
                // Usually EMFILE. Spinning on it would starve everything else,
                // which is what axum's own loop does about it too.
                tracing::warn!(error = %e, "accept failed");
                tokio::time::sleep(Duration::from_millis(50)).await;
                continue;
            }
        };
        let Ok(permit) = permits.clone().try_acquire_owned() else {
            tracing::debug!(%peer, "too many handshakes in progress; dropping");
            continue;
        };
        let acceptor = acceptor.clone();
        let handed_off = ready.clone();
        tokio::spawn(async move {
            let _permit = permit;
            match tokio::time::timeout(handshake_timeout, acceptor.accept(socket)).await {
                Ok(Ok(stream)) => {
                    // The axum side is gone only when the relay is shutting
                    // down; the stream is dropped with it.
                    let _ = handed_off.send((stream, peer)).await;
                }
                Ok(Err(e)) => tracing::debug!(%peer, error = %e, "TLS handshake failed"),
                Err(_) => tracing::debug!(%peer, "TLS handshake timed out"),
            }
        });
        if ready.is_closed() {
            return;
        }
    }
}

impl axum::serve::Listener for TlsListener {
    type Io = TlsStream<TcpStream>;
    type Addr = SocketAddr;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        match self.ready.recv().await {
            Some(accepted) => accepted,
            // The accept loop only ends once this receiver is gone, so a
            // closed channel here cannot happen while `self` is alive.
            None => std::future::pending().await,
        }
    }

    fn local_addr(&self) -> std::io::Result<Self::Addr> {
        Ok(self.local)
    }
}
