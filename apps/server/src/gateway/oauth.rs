//! OAuth for integrations: Storm as an OAuth **client** of an upstream
//! (spec §10, AM25; decision 81g).
//!
//! rmcp's `AuthorizationManager` does the protocol — RFC 9728 → RFC 8414
//! discovery, RFC 7591 dynamic registration as a public client, PKCE S256,
//! `resource` (RFC 8707), the code exchange and the refresh. Everything it
//! touches goes through pieces of ours:
//!
//! - [`SsrfHttp`], the only HTTP client it gets. **Discovery never targets
//!   loopback, link-local or private ranges** (§10): every URL, and every
//!   redirect hop, is resolved first, refused unless every address is public,
//!   and then connected to *those* addresses, so a DNS answer cannot change
//!   between the check and the connect. https only.
//! - [`TokenStore`], rmcp's `CredentialStore` over `gateway.db`: the token set
//!   is sealed like any credential, under kind `oauth_tokens`. rmcp saves a
//!   refreshed pair through it **before** returning the new access token, so a
//!   rotated refresh token is on disk before it is used.
//! - [`FlowStore`], rmcp's `StateStore` over `oauth_flows`: the `state` is
//!   stored only as its blake3 hash, the PKCE verifier sealed, ten minutes,
//!   and **single use**: loading a flow claims it, atomically.
//!
//! The browser returns to the *client* (G-D13), which relays `{state, code}`
//! to `POST /v1/integrations/oauth/callback`. A LAN `http` redirect is refused
//! by the upstreams that matter (G1), so the only redirect URIs accepted are
//! loopback (desktop) and `storm://oauth` (Android, macOS).

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use rmcp::transport::auth::{
    AuthError, AuthorizationManager, AuthorizationMetadata, AuthorizationMetadataSource,
    CredentialStore, OAuthClientConfig, OAuthHttpClient, OAuthHttpClientError,
    OAuthHttpClientFuture, OAuthHttpRedirectPolicy, OAuthHttpRequest, StateStore,
    StoredAuthorizationState, StoredCredentials,
};

use super::Gateway;
use super::connections::credential_kind;

/// A flow's life (§8).
pub const FLOW_TTL_SECS: i64 = 600;
const MAX_REDIRECTS: usize = 3;
const OAUTH_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_BODY: usize = 1024 * 1024;

// ---- the SSRF rule ----------------------------------------------------------

/// Whether an address is one discovery may reach: not loopback, private,
/// link-local, shared (CGNAT), unspecified, multicast or documentation space.
pub fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            !(v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || v4.is_multicast()
                || v4.is_documentation()
                || o[0] == 0
                || (o[0] == 100 && (64..128).contains(&o[1])) // 100.64/10
                || (o[0] == 192 && o[1] == 0 && o[2] == 0) // 192.0.0/24
                || (o[0] == 198 && (18..20).contains(&o[1])) // 198.18/15
                || o[0] >= 240)
        }
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_public(IpAddr::V4(v4));
            }
            let s = v6.segments();
            !(v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || (s[0] & 0xfe00) == 0xfc00 // unique local
                || (s[0] & 0xffc0) == 0xfe80 // link local
                || (s[0] == 0x2001 && s[1] == 0x0db8)) // documentation
        }
    }
}

/// Why a URL may not be fetched. The text is for the owner; it names the
/// rule, never what the upstream said.
#[derive(Debug, thiserror::Error)]
pub enum SsrfError {
    #[error("integration URLs for OAuth must be https")]
    NotHttps,
    #[error("the URL has no host")]
    NoHost,
    #[error("the host does not resolve")]
    Unresolvable,
    #[error("the host resolves to a loopback, private or link-local address")]
    NotPublic,
}

/// Resolves `url`'s host and checks every address. Returns what to connect
/// to. `allow_private` is the test suites' switch (and `http`).
pub async fn check_target(
    url: &url::Url,
    allow_private: bool,
) -> Result<(String, Vec<SocketAddr>), SsrfError> {
    match url.scheme() {
        "https" => {}
        "http" if allow_private => {}
        _ => return Err(SsrfError::NotHttps),
    }
    let host = url.host_str().ok_or(SsrfError::NoHost)?.to_string();
    let port = url.port_or_known_default().unwrap_or(443);
    let bare = host.trim_start_matches('[').trim_end_matches(']');
    let addrs: Vec<SocketAddr> = match bare.parse::<IpAddr>() {
        Ok(ip) => vec![SocketAddr::new(ip, port)],
        Err(_) => tokio::net::lookup_host((bare, port))
            .await
            .map_err(|_| SsrfError::Unresolvable)?
            .collect(),
    };
    if addrs.is_empty() {
        return Err(SsrfError::Unresolvable);
    }
    // Every address, not any: a name with one public and one private answer
    // is refused, because the connect could pick either.
    if !allow_private && addrs.iter().any(|a| !is_public(a.ip())) {
        return Err(SsrfError::NotPublic);
    }
    Ok((host, addrs))
}

/// The only HTTP client rmcp's OAuth code gets.
pub struct SsrfHttp {
    pub allow_private: bool,
}

impl SsrfHttp {
    async fn run(
        &self,
        request: OAuthHttpRequest,
    ) -> Result<axum::http::Response<Vec<u8>>, OAuthHttpClientError> {
        let OAuthHttpRequest {
            request,
            redirect_policy,
            ..
        } = request;
        let (parts, body) = request.into_parts();
        let mut method = parts.method;
        let mut uri = parts.uri;
        let mut headers = parts.headers;
        let mut body = body;
        for _ in 0..=MAX_REDIRECTS {
            let url = url::Url::parse(&uri.to_string())?;
            let (host, addrs) = check_target(&url, self.allow_private).await?;
            let client = pinned_client(&host, &addrs)?;
            let mut req = client.request(method.clone(), url.clone());
            for (name, value) in headers.iter() {
                req = req.header(name, value);
            }
            let response = req.body(body.clone()).send().await?;
            let status = response.status();
            if status.is_redirection() && redirect_policy == OAuthHttpRedirectPolicy::Follow {
                let Some(location) = response
                    .headers()
                    .get(reqwest::header::LOCATION)
                    .and_then(|l| l.to_str().ok())
                else {
                    return collect(response).await;
                };
                let next = url.join(location)?;
                if next.origin() != url.origin() {
                    // Never carry a credential to another origin.
                    headers.remove(reqwest::header::AUTHORIZATION);
                }
                if !matches!(status.as_u16(), 307 | 308) {
                    method = reqwest::Method::GET;
                    body = Vec::new();
                    headers.remove(reqwest::header::CONTENT_TYPE);
                }
                uri = next.as_str().parse()?;
                continue;
            }
            return collect(response).await;
        }
        Err("too many redirects".into())
    }
}

/// A client for one request under the SSRF rule: **pinned to the checked
/// addresses**, so the connect cannot re-resolve, and following no redirect
/// itself (each hop is checked by the caller).
fn pinned_client(host: &str, addrs: &[SocketAddr]) -> reqwest::Result<reqwest::Client> {
    reqwest::Client::builder()
        .use_preconfigured_tls(super::upstream::tls_config())
        .redirect(reqwest::redirect::Policy::none())
        .timeout(OAUTH_TIMEOUT)
        .resolve_to_addrs(host, addrs)
        .build()
}

async fn collect(
    response: reqwest::Response,
) -> Result<axum::http::Response<Vec<u8>>, OAuthHttpClientError> {
    use futures_util::StreamExt;
    let mut builder = axum::http::Response::builder().status(response.status().as_u16());
    for (name, value) in response.headers() {
        builder = builder.header(name.as_str(), value.as_bytes());
    }
    let mut body = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        if body.len() + chunk.len() > MAX_BODY {
            return Err("an OAuth response over 1 MiB".into());
        }
        body.extend_from_slice(&chunk);
    }
    Ok(builder.body(body)?)
}

impl OAuthHttpClient for SsrfHttp {
    fn execute(&self, request: OAuthHttpRequest) -> OAuthHttpClientFuture<'_> {
        Box::pin(self.run(request))
    }
}

// ---- the stores ---------------------------------------------------------------

/// rmcp's credential store for one connection: sealed in `credentials`.
pub struct TokenStore {
    pub gateway: Arc<Gateway>,
    pub connection: String,
}

impl TokenStore {
    pub fn new(gateway: &Arc<Gateway>, c: &super::connections::Connection) -> Self {
        Self {
            gateway: gateway.clone(),
            connection: c.id.clone(),
        }
    }
}

fn store_error(e: impl std::fmt::Display) -> AuthError {
    AuthError::InternalError(e.to_string())
}

#[async_trait]
impl CredentialStore for TokenStore {
    async fn load(&self) -> Result<Option<StoredCredentials>, AuthError> {
        let row = self
            .gateway
            .store
            .lock()
            .expect("gateway store lock")
            .credential(&self.connection, credential_kind::OAUTH_TOKENS)
            .map_err(store_error)?;
        let Some((sealed, _)) = row else {
            return Ok(None);
        };
        self.gateway
            .keys
            .open_json(&self.connection, credential_kind::OAUTH_TOKENS, &sealed)
            .map(Some)
            .map_err(store_error)
    }

    async fn save(&self, credentials: StoredCredentials) -> Result<(), AuthError> {
        let sealed = self
            .gateway
            .keys
            .seal_json(
                &self.connection,
                credential_kind::OAUTH_TOKENS,
                &credentials,
            )
            .map_err(store_error)?;
        self.gateway
            .store
            .lock()
            .expect("gateway store lock")
            .put_credential(
                &self.connection,
                credential_kind::OAUTH_TOKENS,
                &sealed,
                None,
                &crate::index::now_rfc3339(),
            )
            .map_err(store_error)
    }

    async fn clear(&self) -> Result<(), AuthError> {
        self.gateway
            .store
            .lock()
            .expect("gateway store lock")
            .delete_credential(&self.connection, credential_kind::OAUTH_TOKENS)
            .map(|_| ())
            .map_err(store_error)
    }
}

pub fn state_hash(state: &str) -> String {
    blake3::hash(state.as_bytes()).to_hex().to_string()
}

/// rmcp's state store: `oauth_flows`, the state hashed, the verifier sealed.
pub struct FlowStore {
    pub gateway: Arc<Gateway>,
    pub connection: String,
    pub owner: String,
    pub oauth_client: String,
    pub redirect_uri: String,
    pub resource: String,
}

impl FlowStore {
    /// The flow store for authorizing connection `c` with `client`, whose
    /// owner is the connection's.
    pub fn new(
        gateway: &Arc<Gateway>,
        c: &super::connections::Connection,
        client: &super::store::OAuthClientRow,
    ) -> Self {
        Self {
            gateway: gateway.clone(),
            connection: c.id.clone(),
            owner: c.owner_user_id.clone(),
            oauth_client: client.id.clone(),
            redirect_uri: client.redirect_uri.clone(),
            resource: c.url.clone(),
        }
    }
}

const PKCE_KIND: &str = "pkce_verifier";

#[async_trait]
impl StateStore for FlowStore {
    async fn save(
        &self,
        csrf_token: &str,
        state: StoredAuthorizationState,
    ) -> Result<(), AuthError> {
        let hash = state_hash(csrf_token);
        let sealed = self
            .gateway
            .keys
            .seal_json(&hash, PKCE_KIND, &state)
            .map_err(store_error)?;
        let now = crate::index::now_rfc3339();
        let expires = super::rfc3339_in(FLOW_TTL_SECS);
        self.gateway
            .store
            .lock()
            .expect("gateway store lock")
            .insert_flow(&super::store::FlowRow {
                state_hash: hash,
                connection_id: self.connection.clone(),
                owner_user_id: self.owner.clone(),
                oauth_client: self.oauth_client.clone(),
                redirect_uri: self.redirect_uri.clone(),
                resource: self.resource.clone(),
                sealed,
                created_at: now,
                expires_at: expires,
            })
            .map_err(store_error)
    }

    /// **Claims** the flow: single use, decided atomically in SQL, so two
    /// callbacks racing on one `state` cannot both exchange its code.
    async fn load(&self, csrf_token: &str) -> Result<Option<StoredAuthorizationState>, AuthError> {
        let hash = state_hash(csrf_token);
        let claimed = self
            .gateway
            .store
            .lock()
            .expect("gateway store lock")
            .claim_flow(&hash, &crate::index::now_rfc3339())
            .map_err(store_error)?;
        let Some(row) = claimed else {
            return Ok(None);
        };
        if row.connection_id != self.connection {
            return Ok(None);
        }
        self.gateway
            .keys
            .open_json(&hash, PKCE_KIND, &row.sealed)
            .map(Some)
            .map_err(store_error)
    }

    async fn delete(&self, _csrf_token: &str) -> Result<(), AuthError> {
        // Already spent by `load`.
        Ok(())
    }
}

// ---- redirect URIs --------------------------------------------------------------

/// The redirect URIs a client may ask for (G-D13, G1): an `http` loopback
/// (`127.0.0.1`, `[::1]` or `localhost`, any port) for desktop, or
/// `storm://oauth…` for Android and macOS. Never a LAN or public `http` URL.
pub fn validate_redirect(uri: &str) -> Result<(), &'static str> {
    let Ok(url) = url::Url::parse(uri) else {
        return Err("the redirect URI does not parse");
    };
    if url.fragment().is_some() {
        return Err("the redirect URI must not have a fragment");
    }
    match url.scheme() {
        "storm" if url.host_str() == Some("oauth") => Ok(()),
        "http" if matches!(url.host_str(), Some("127.0.0.1" | "[::1]" | "localhost")) => Ok(()),
        _ => Err("the redirect URI must be a loopback address or storm://oauth"),
    }
}

// ---- the manager ----------------------------------------------------------------

/// A manager for one connection, wired to our client and stores, with
/// discovery run. Discovered metadata must be published by the upstream: the
/// legacy fallback (synthesized endpoints) is refused.
pub async fn discovered_manager(
    gateway: &Arc<Gateway>,
    url: &str,
) -> Result<(AuthorizationManager, AuthorizationMetadata), OAuthFailure> {
    let http = Arc::new(SsrfHttp {
        allow_private: gateway.allow_http_upstreams(),
    });
    let mut manager = AuthorizationManager::new_with_oauth_http_client(url, http)
        .await
        .map_err(|_| OAuthFailure::Discovery)?;
    let resolution = manager
        .resolve_metadata()
        .await
        .map_err(|_| OAuthFailure::Discovery)?;
    if resolution.source == AuthorizationMetadataSource::LegacyEndpointFallback {
        return Err(OAuthFailure::NoOAuth);
    }
    manager.set_metadata(resolution.metadata.clone());
    Ok((manager, resolution.metadata))
}

fn client_config(client_id: &str, redirect_uri: &str, secret: Option<String>) -> OAuthClientConfig {
    let config = OAuthClientConfig::new(client_id, redirect_uri);
    match secret {
        Some(s) => config.with_client_secret(s),
        None => config,
    }
}

/// Stable failure codes; the upstream's text never leaves this module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OAuthFailure {
    Discovery,
    NoOAuth,
    NeedsClient,
    Registration,
    Exchange,
    FlowSpent,
}

impl OAuthFailure {
    pub fn code(self) -> &'static str {
        match self {
            OAuthFailure::Discovery => "oauth_discovery_failed",
            OAuthFailure::NoOAuth => "oauth_not_offered",
            OAuthFailure::NeedsClient => "oauth_client_required",
            OAuthFailure::Registration => "oauth_registration_failed",
            OAuthFailure::Exchange => "oauth_exchange_failed",
            OAuthFailure::FlowSpent => "oauth_flow_expired_or_used",
        }
    }
}

/// When a stored token set needs refreshing: within a minute of its expiry.
pub fn needs_refresh(credentials: &StoredCredentials) -> bool {
    use oauth2::TokenResponse;
    let Some(response) = credentials.token_response.as_ref() else {
        return true;
    };
    match (response.expires_in(), credentials.token_received_at) {
        (Some(expires), Some(at)) => {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            now.saturating_sub(at) + 60 >= expires.as_secs()
        }
        _ => false,
    }
}

// ---- the gateway's use of a token set ------------------------------------------

impl Gateway {
    fn refresh_lock(&self, connection: &str) -> Arc<tokio::sync::Mutex<()>> {
        self.refresh_locks
            .lock()
            .expect("refresh locks")
            .entry(connection.to_string())
            .or_default()
            .clone()
    }

    /// The pasted client secret of an OAuth client, opened.
    fn client_secret(&self, client: &super::store::OAuthClientRow) -> Option<String> {
        let sealed = client.secret.as_ref()?;
        let opened = self.keys.open(&client.id, "client_secret", sealed).ok()?;
        String::from_utf8(opened.expose().to_vec()).ok()
    }

    /// Points `manager` at a stored OAuth client: its id, its redirect URI
    /// and any pasted secret.
    pub fn configure_client(
        &self,
        manager: &mut AuthorizationManager,
        client: &super::store::OAuthClientRow,
    ) -> Result<(), AuthError> {
        manager.configure_client(client_config(
            &client.client_id,
            &client.redirect_uri,
            self.client_secret(client),
        ))
    }

    /// The OAuth client connection `connection` was authorized with.
    fn oauth_client_of(
        &self,
        connection: &str,
    ) -> anyhow::Result<Option<super::store::OAuthClientRow>> {
        let store = self.store.lock().expect("gateway store lock");
        match store.connection_oauth(connection)? {
            Some(id) => store.oauth_client(&id),
            None => Ok(None),
        }
    }

    /// An `oauth` connection's access token, refreshed first when it is within
    /// a minute of expiry, or when `force` (an upstream refused it).
    ///
    /// **Single flight per connection**: refreshes are serialized by a lock
    /// held across the refresh, and the token set is re-read under it, so two
    /// calls that both found it expired refresh it once. rmcp saves the new
    /// pair through [`TokenStore`] before returning it, so a rotated refresh
    /// token is persisted before the access token is used.
    pub async fn oauth_access_token(
        self: &Arc<Self>,
        c: &super::connections::Connection,
        force: bool,
    ) -> Result<String, super::upstream::UpstreamError> {
        use super::upstream::UpstreamError;
        use oauth2::TokenResponse;
        let tokens = TokenStore::new(self, c);
        let current = |s: &StoredCredentials| {
            s.token_response
                .as_ref()
                .map(|t| t.access_token().secret().to_string())
        };
        let stored = tokens
            .load()
            .await
            .map_err(|_| UpstreamError::Credential)?
            .ok_or(UpstreamError::Credential)?;
        if !force && !needs_refresh(&stored) {
            return current(&stored).ok_or(UpstreamError::Credential);
        }
        let lock = self.refresh_lock(&c.id);
        let _held = lock.lock().await;
        let stored = tokens
            .load()
            .await
            .map_err(|_| UpstreamError::Credential)?
            .ok_or(UpstreamError::Credential)?;
        if !needs_refresh(&stored) && !force {
            return current(&stored).ok_or(UpstreamError::Credential);
        }
        let client = self
            .oauth_client_of(&c.id)
            .map_err(|_| UpstreamError::Credential)?
            .ok_or(UpstreamError::Credential)?;
        let (mut manager, _) = discovered_manager(self, &c.url)
            .await
            .map_err(|_| UpstreamError::Unavailable)?;
        self.configure_client(&mut manager, &client)
            .map_err(|_| UpstreamError::Credential)?;
        manager.set_credential_store(tokens);
        match manager.refresh_token().await {
            Ok(fresh) => Ok(fresh.access_token().secret().to_string()),
            Err(AuthError::TokenRefreshRejected(_) | AuthError::AuthorizationRequired) => {
                Err(UpstreamError::Credential)
            }
            Err(_) => Err(UpstreamError::Unavailable),
        }
    }
}

/// What revocation needs, read before a disconnect deletes it.
pub struct Revocation {
    endpoint: String,
    client_id: String,
    token: String,
    hint: &'static str,
}

impl Gateway {
    /// Reads what an RFC 7009 revocation of this connection would send, if its
    /// authorization server offers one. `None` for a static token (a GitHub
    /// PAT is revoked at GitHub, §13) or a server without the endpoint.
    pub async fn revocation_for(
        self: &Arc<Self>,
        c: &super::connections::Connection,
    ) -> Option<Revocation> {
        use oauth2::TokenResponse;
        let stored = TokenStore::new(self, c).load().await.ok()??;
        let client = self.oauth_client_of(&c.id).ok()??;
        let metadata: serde_json::Value = serde_json::from_str(client.metadata.as_deref()?).ok()?;
        let endpoint = metadata.get("revocation_endpoint")?.as_str()?.to_string();
        let response = stored.token_response?;
        let (token, hint) = match response.refresh_token() {
            Some(r) => (r.secret().to_string(), "refresh_token"),
            None => (response.access_token().secret().to_string(), "access_token"),
        };
        Some(Revocation {
            endpoint,
            client_id: client.client_id,
            token,
            hint,
        })
    }

    /// Best effort (§13): one POST, under the SSRF rule, never retried, and
    /// nothing waits on it.
    pub async fn revoke(&self, r: Revocation) {
        let Ok(url) = url::Url::parse(&r.endpoint) else {
            return;
        };
        let Ok((host, addrs)) = check_target(&url, self.allow_http_upstreams()).await else {
            return;
        };
        let Ok(client) = pinned_client(&host, &addrs) else {
            return;
        };
        let body = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("token", &r.token)
            .append_pair("token_type_hint", r.hint)
            .append_pair("client_id", &r.client_id)
            .finish();
        let _ = client
            .post(url)
            .header(
                reqwest::header::CONTENT_TYPE,
                "application/x-www-form-urlencoded",
            )
            .body(body)
            .send()
            .await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_public_addresses_are_public() {
        for private in [
            "127.0.0.1",
            "10.1.2.3",
            "172.16.0.1",
            "192.168.1.51",
            "169.254.169.254",
            "100.64.0.1",
            "0.0.0.0",
            "::1",
            "fd00::1",
            "fe80::1",
            "::ffff:192.168.1.1",
            "224.0.0.1",
        ] {
            assert!(!is_public(private.parse().unwrap()), "{private}");
        }
        for public in ["1.1.1.1", "140.82.112.3", "2606:4700::1111"] {
            assert!(is_public(public.parse().unwrap()), "{public}");
        }
    }

    #[tokio::test]
    async fn discovery_never_targets_a_private_or_plain_http_url() {
        let check = |u: &str| url::Url::parse(u).unwrap();
        assert!(matches!(
            check_target(&check("http://example.com/x"), false).await,
            Err(SsrfError::NotHttps)
        ));
        for u in [
            "https://127.0.0.1/.well-known/oauth-authorization-server",
            "https://[::1]:8443/x",
            "https://169.254.169.254/latest/meta-data",
            "https://192.168.1.51:8484/x",
            "https://localhost/x",
        ] {
            assert!(
                matches!(
                    check_target(&check(u), false).await,
                    Err(SsrfError::NotPublic)
                ),
                "{u}"
            );
        }
        // The test suites' switch lifts both rules, and nothing else does.
        assert!(
            check_target(&check("http://127.0.0.1:9/x"), true)
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn the_oauth_client_refuses_a_private_target_before_connecting() {
        // A listener that records whether anything connected.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let hit = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let seen = hit.clone();
        tokio::spawn(async move {
            if listener.accept().await.is_ok() {
                seen.store(true, std::sync::atomic::Ordering::SeqCst);
            }
        });
        let client = SsrfHttp {
            allow_private: false,
        };
        // rmcp builds these; a test builds one through its discovery path.
        let manager = AuthorizationManager::new_with_oauth_http_client(
            format!("https://127.0.0.1:{port}/mcp"),
            Arc::new(client),
        )
        .await
        .unwrap();
        // Refused or fallen back, either way nothing may have connected.
        let _ = manager.resolve_metadata().await;
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(
            !hit.load(std::sync::atomic::Ordering::SeqCst),
            "discovery connected to a loopback address"
        );
    }

    /// Network, so not in CI: `cargo test -- --ignored real_discovery`. G1's
    /// three upstreams through the SSRF-checked client: Notion and Linear
    /// publish metadata with dynamic registration; GitHub publishes metadata
    /// without it, which is why it is a PAT connection in V1 (G-D24).
    #[tokio::test]
    #[ignore]
    async fn real_discovery_resolves_g1s_upstreams_through_the_ssrf_client() {
        let dir = tempdir::TempDir::new("storm-oauth-real").unwrap();
        let gw = Arc::new(Gateway::open(dir.path(), "t").unwrap());
        for (url, dcr) in [
            ("https://mcp.notion.com/mcp", true),
            ("https://mcp.linear.app/mcp", true),
            (concat!("https://api.git", "hubcopilot.com/mcp/"), false),
        ] {
            let (_, metadata) = discovered_manager(&gw, url)
                .await
                .unwrap_or_else(|e| panic!("{url}: {e:?}"));
            assert_eq!(metadata.registration_endpoint.is_some(), dcr, "{url}");
        }
    }

    #[test]
    fn redirects_are_loopback_or_storm_only() {
        for good in [
            "http://127.0.0.1:53682/callback",
            "http://localhost:9000/cb",
            "http://[::1]:1/x",
            "storm://oauth/callback",
        ] {
            assert!(validate_redirect(good).is_ok(), "{good}");
        }
        for bad in [
            "http://192.168.1.51:8484/oauth",
            "http://storm.lan/cb",
            "https://example.com/cb",
            "storm://other/cb",
            "storm://oauth/cb#frag",
            "javascript:alert(1)",
        ] {
            assert!(validate_redirect(bad).is_err(), "{bad}");
        }
    }
}
