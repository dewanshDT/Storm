//! The gateway's MCP client to an upstream (spec §9, decision 81d).
//!
//! **Only `*_once` request methods.** rmcp's `call_tool`, `get_prompt` and
//! `read_resource` re-send their request to drive SEP-2322 "input required"
//! rounds, which turns one agent call into several upstream executions
//! (G3). The `*_once` methods send exactly one request. The tests at the
//! bottom of this file read the gateway's source and fail on any other.
//!
//! **The HTTP client is ours, not rmcp's default:**
//! - redirects are never followed, so a credential header is never replayed
//!   to a host it was not configured for (AM24);
//! - TLS is rustls on `ring` with the bundled Mozilla roots, the same choice
//!   the relay tunnel made: the release binary is static musl and must not
//!   depend on the box's `/etc/ssl`, and it must not depend on a process-wide
//!   provider having been installed first;
//! - a connect timeout bounds a dead upstream. There is deliberately no
//!   whole-request timeout on the client: it would also cut the standalone
//!   SSE stream. Each operation carries its own deadline instead (the
//!   owner's probe here; the spec's 60 s per agent call with the route).

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use rmcp::model::{ClientCapabilities, ClientInfo, Implementation, Tool};
use rmcp::service::{RoleClient, RunningService};
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::transport::streamable_http_client::{
    StreamableHttpClientTransportConfig, StreamableHttpError,
};
use rmcp::{ClientHandler, ServiceExt};

use super::connections::{Connection, StaticCredential, auth_kind, credential_kind};

/// How long a connect (TCP + TLS) may take.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// The owner's test and tool listing are interactive; they give up sooner.
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(30);

/// Why an upstream call failed, as the stable codes of spec §12. **Never the
/// upstream's own error text**: it can echo a request, and it reaches the
/// owner's client and the audit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpstreamError {
    /// 401, or 403 with a challenge: the credential is no good.
    Unauthorized,
    /// 429.
    RateLimited,
    /// Connect failure, timeout, 5xx, a broken stream.
    Unavailable,
    /// The upstream answered, but not as an MCP server.
    Protocol,
    /// The connection's own credential is missing or cannot be opened.
    Credential,
}

impl UpstreamError {
    pub fn code(self) -> &'static str {
        match self {
            UpstreamError::Unauthorized => "upstream_unauthorized",
            UpstreamError::RateLimited => "upstream_rate_limited",
            UpstreamError::Unavailable => "upstream_unavailable",
            UpstreamError::Protocol => "upstream_protocol_error",
            UpstreamError::Credential => "integration_needs_reauth",
        }
    }

    /// Whether this failure means the owner must reconnect.
    pub fn needs_reauth(self) -> bool {
        matches!(
            self,
            UpstreamError::Unauthorized | UpstreamError::Credential
        )
    }
}

/// Where to send a connection's requests and what to present. Built per call
/// from the sealed credential, and dropped with it; never stored, never
/// logged (the `Debug` below names header *names* only).
pub struct Target {
    pub url: String,
    pub headers: HashMap<axum::http::HeaderName, axum::http::HeaderValue>,
}

impl std::fmt::Debug for Target {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let names: Vec<&str> = self.headers.keys().map(|h| h.as_str()).collect();
        f.debug_struct("Target")
            .field("url", &self.url)
            .field("headers", &names)
            .finish()
    }
}

impl super::Gateway {
    /// The target for a connection: its URL and its credential as a header —
    /// a `static` connection's opened value, or an `oauth` connection's access
    /// token, refreshed first when it is about to expire (81g). `force`
    /// refreshes regardless: an upstream just refused the token.
    pub async fn target(
        self: &Arc<Self>,
        c: &Connection,
        force: bool,
    ) -> Result<Target, UpstreamError> {
        let mut headers = HashMap::new();
        match c.auth_kind.as_str() {
            auth_kind::NONE => {}
            auth_kind::OAUTH => {
                let token = self.oauth_access_token(c, force).await?;
                let mut value = axum::http::HeaderValue::from_str(&format!("Bearer {token}"))
                    .map_err(|_| UpstreamError::Credential)?;
                value.set_sensitive(true);
                headers.insert(axum::http::header::AUTHORIZATION, value);
            }
            auth_kind::STATIC => {
                let sealed = self
                    .store
                    .lock()
                    .expect("gateway store lock")
                    .credential(&c.id, credential_kind::STATIC)
                    .map_err(|_| UpstreamError::Credential)?
                    .ok_or(UpstreamError::Credential)?
                    .0;
                let opened = self
                    .keys
                    .open(&c.id, credential_kind::STATIC, &sealed)
                    .map_err(|_| UpstreamError::Credential)?;
                let credential: StaticCredential = serde_json::from_slice(opened.expose())
                    .map_err(|_| UpstreamError::Credential)?;
                let name = axum::http::HeaderName::from_bytes(credential.header.as_bytes())
                    .map_err(|_| UpstreamError::Credential)?;
                let mut value = axum::http::HeaderValue::from_str(&credential.value)
                    .map_err(|_| UpstreamError::Credential)?;
                // Kept out of HTTP-level debug output, where hyper honours it.
                value.set_sensitive(true);
                headers.insert(name, value);
            }
            _ => return Err(UpstreamError::Credential),
        }
        Ok(Target {
            url: c.url.clone(),
            headers,
        })
    }
}

/// rustls on `ring` with the bundled Mozilla roots: every TLS connection the
/// gateway makes, to an upstream or to an authorization server.
pub fn tls_config() -> rustls::ClientConfig {
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    rustls::ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .with_safe_default_protocol_versions()
        .expect("ring supports the default TLS versions")
        .with_root_certificates(roots)
        .with_no_client_auth()
}

/// The one HTTP client for every upstream, built once.
pub fn http_client() -> reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            reqwest::Client::builder()
                .use_preconfigured_tls(tls_config())
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(CONNECT_TIMEOUT)
                // rmcp's own default, for its reason: a pooled connection whose
                // previous body was not fully read stalls ~40 ms on Linux.
                .pool_max_idle_per_host(0)
                .build()
                .expect("the gateway's HTTP client builds")
        })
        .clone()
}

/// The identity the gateway presents upstream. Capabilities come from the
/// caller: for the owner's probes, none at all; for an agent's session
/// (81e), the agent's own minus sampling, roots and `elicitation.url`.
pub fn client_info(capabilities: ClientCapabilities) -> ClientInfo {
    ClientInfo::new(
        capabilities,
        Implementation::new("storm-gateway", env!("CARGO_PKG_VERSION")),
    )
}

/// Opens an MCP session to `target` with `handler`.
pub async fn connect<H: ClientHandler>(
    target: Target,
    handler: H,
) -> Result<RunningService<RoleClient, H>, UpstreamError> {
    let config = StreamableHttpClientTransportConfig::with_uri(target.url)
        .custom_headers(target.headers)
        // rmcp's default, kept on purpose and named so it is not "tidied"
        // off: an upstream 404 re-initializes and the call runs exactly once
        // (G3, R3b).
        .reinit_on_expired_session(true);
    let transport = StreamableHttpClientTransport::with_client(http_client(), config);
    handler.serve(transport).await.map_err(|e| classify(&e))
}

/// A probe's handler: no capabilities, so an upstream may send it nothing.
#[derive(Clone)]
pub struct Probe;

impl ClientHandler for Probe {
    fn get_info(&self) -> ClientInfo {
        client_info(ClientCapabilities::default())
    }
}

/// What the owner's test learns.
pub struct ProbeResult {
    pub server_name: String,
    pub server_version: String,
    pub tools: Vec<Tool>,
}

/// Connects, lists every tool, and closes — the owner's `test` and `tools`
/// (§14). One short-lived session, never an agent's.
pub async fn probe(target: Target) -> Result<ProbeResult, UpstreamError> {
    let work = async {
        let client = connect(target, Probe).await?;
        let info = client.peer_info();
        let tools = client.list_all_tools().await.map_err(|e| classify(&e));
        let _ = client.cancel().await;
        let tools = tools?;
        Ok(ProbeResult {
            server_name: info
                .as_ref()
                .and_then(|i| i.server_info.as_ref())
                .map(|s| s.name.clone())
                .unwrap_or_default(),
            server_version: info
                .as_ref()
                .and_then(|i| i.server_info.as_ref())
                .map(|s| s.version.clone())
                .unwrap_or_default(),
            tools,
        })
    };
    tokio::time::timeout(PROBE_TIMEOUT, work)
        .await
        .unwrap_or(Err(UpstreamError::Unavailable))
}

/// Maps whatever rmcp returned to a stable code, by walking the error's
/// sources for the transport's own error. Unknown shapes are `Unavailable`:
/// a code the owner can retry, never one that marks a working credential bad.
pub fn classify(error: &(dyn std::error::Error + 'static)) -> UpstreamError {
    let mut current: Option<&(dyn std::error::Error + 'static)> = Some(error);
    while let Some(e) = current {
        if let Some(http) = e.downcast_ref::<StreamableHttpError<reqwest::Error>>() {
            return classify_http(http);
        }
        // Neither rmcp error marks its transport error as a `source`, so the
        // walk steps into them by hand.
        if let Some(service) = e.downcast_ref::<rmcp::ServiceError>() {
            match service {
                rmcp::ServiceError::UnexpectedResponse => return UpstreamError::Protocol,
                rmcp::ServiceError::TransportSend(t) => {
                    current = Some(t.error.as_ref());
                    continue;
                }
                _ => {}
            }
        }
        if let Some(init) = e.downcast_ref::<rmcp::service::ClientInitializeError>() {
            use rmcp::service::ClientInitializeError as I;
            match init {
                I::ExpectedInitResponse(_)
                | I::ExpectedInitResult(_)
                | I::ConflictInitResponseId(..)
                | I::JsonRpcError(_)
                | I::NoCompatibleProtocolVersion { .. } => return UpstreamError::Protocol,
                I::TransportError { error, .. } => {
                    current = Some(error.error.as_ref());
                    continue;
                }
                _ => {}
            }
        }
        current = e.source();
    }
    UpstreamError::Unavailable
}

fn classify_http(e: &StreamableHttpError<reqwest::Error>) -> UpstreamError {
    match e {
        StreamableHttpError::AuthRequired(_) | StreamableHttpError::InsufficientScope(_) => {
            UpstreamError::Unauthorized
        }
        StreamableHttpError::UnexpectedServerResponse(text) => {
            // rmcp formats a non-success answer as "HTTP <status>: <body>".
            // Only the status is read; the body never leaves this function.
            let status = text
                .strip_prefix("HTTP ")
                .and_then(|rest| rest.get(..3))
                .and_then(|code| code.parse::<u16>().ok());
            match status {
                Some(401) | Some(403) => UpstreamError::Unauthorized,
                Some(429) => UpstreamError::RateLimited,
                Some(s) if s >= 500 => UpstreamError::Unavailable,
                Some(_) => UpstreamError::Protocol,
                None => UpstreamError::Unavailable,
            }
        }
        StreamableHttpError::UnexpectedContentType(_)
        | StreamableHttpError::Deserialize(_)
        | StreamableHttpError::MissingSessionIdInResponse
        | StreamableHttpError::ServerDoesNotSupportSse => UpstreamError::Protocol,
        _ => UpstreamError::Unavailable,
    }
}

/// The allowlist rule (G-D16), applied to a fresh listing. Returns the tools
/// that are new to the owner.
///
/// - **The first listing turns everything on**, and remembers it.
/// - **Later, a tool not seen before stays off** until the owner turns it on,
///   and is reported as new so the Integrations screen can say so.
/// - A tool the owner already decided on keeps that decision, even if it
///   vanished upstream and came back: `known_tools` only grows.
pub fn reconcile_tools(c: &mut Connection, listed: &[String]) -> Vec<String> {
    let mut listed: Vec<String> = listed.to_vec();
    listed.sort();
    listed.dedup();
    match &mut c.known_tools {
        None => {
            c.tool_allowlist = listed.clone();
            c.known_tools = Some(listed);
            Vec::new()
        }
        Some(known) => {
            let new: Vec<String> = listed
                .iter()
                .filter(|t| !known.contains(t))
                .cloned()
                .collect();
            known.extend(new.iter().cloned());
            known.sort();
            known.dedup();
            new
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use rmcp::model::{CallToolResult, ListToolsResult, ServerCapabilities, ServerInfo};
    use rmcp::service::RequestContext;
    use rmcp::{ErrorData, RoleServer, ServerHandler};

    /// A stand-in upstream: tools from a shared list, an optional required
    /// header, and a count of every `tools/call` it executed.
    #[derive(Clone)]
    pub(crate) struct MockUpstream {
        pub tools: Arc<std::sync::Mutex<Vec<String>>>,
        pub calls: Arc<std::sync::atomic::AtomicUsize>,
    }

    impl ServerHandler for MockUpstream {
        fn get_info(&self) -> ServerInfo {
            let mut info = ServerInfo::default();
            info.capabilities = ServerCapabilities::builder().enable_tools().build();
            info.server_info = Implementation::new("mock-upstream", "1.2.3");
            info
        }

        async fn list_tools(
            &self,
            _: Option<rmcp::model::PaginatedRequestParams>,
            _: RequestContext<RoleServer>,
        ) -> Result<ListToolsResult, ErrorData> {
            let tools = self
                .tools
                .lock()
                .unwrap()
                .iter()
                .map(|name| {
                    Tool::new(
                        name.clone(),
                        format!("the {name} tool"),
                        Arc::new(serde_json::Map::from_iter([(
                            "type".to_string(),
                            serde_json::json!("object"),
                        )])),
                    )
                })
                .collect();
            Ok(ListToolsResult::with_all_items(tools))
        }

        async fn call_tool(
            &self,
            request: rmcp::model::CallToolRequestParams,
            ctx: RequestContext<RoleServer>,
        ) -> Result<rmcp::model::CallToolResponse, ErrorData> {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let text = match request.name.as_ref() {
                // What the gateway declared upstream (§9: never sampling,
                // roots or `elicitation.url`).
                "caps" => ctx
                    .peer
                    .peer_info()
                    .map(|i| serde_json::to_string(&i.capabilities).unwrap_or_default())
                    .unwrap_or_default(),
                // Two progress notifications on this call's stream, then done.
                "slow" => {
                    if let Some(token) = ctx.meta.get_progress_token() {
                        for n in 1..=2 {
                            let p: rmcp::model::ProgressNotificationParam =
                                serde_json::from_value(serde_json::json!({
                                    "progressToken": token, "progress": n, "total": 2
                                }))
                                .unwrap();
                            let _ = ctx.peer.notify_progress(p).await;
                        }
                    }
                    "slow done".into()
                }
                // A form elicitation (or a URL one), answered by the agent.
                "ask" | "ask_url" => {
                    let params = if request.name == "ask" {
                        serde_json::json!({"mode": "form", "message": "lang?",
                            "requestedSchema": {"type": "object", "properties": {"answer": {"type": "string"}}}})
                    } else {
                        serde_json::json!({"mode": "url", "message": "log in",
                            "url": "https://example.com/login", "elicitationId": "e1"})
                    };
                    let params: rmcp::model::ElicitRequestParams =
                        serde_json::from_value(params).unwrap();
                    let answer = ctx
                        .peer
                        .send_request(rmcp::model::ServerRequest::ElicitRequest(
                            rmcp::model::ElicitRequest::new(params),
                        ))
                        .await;
                    format!(
                        "answer {}",
                        serde_json::to_string(&answer.ok()).unwrap_or_default()
                    )
                }
                // Holds the call open, for the in-flight cases.
                "hang" => {
                    tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                    "late".into()
                }
                name => format!("called {name}"),
            };
            Ok(CallToolResult::success(vec![rmcp::model::ContentBlock::text(text)]).into())
        }
    }

    /// Serves `upstream` on 127.0.0.1, refusing any request whose `header`
    /// is not `expected` with a 401. Returns its `http://` URL.
    pub(crate) async fn serve_mock(
        upstream: MockUpstream,
        required: Option<(&'static str, String)>,
    ) -> String {
        use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
        use rmcp::transport::streamable_http_server::{
            StreamableHttpServerConfig, StreamableHttpService,
        };
        let service = StreamableHttpService::new(
            move || Ok(upstream.clone()),
            Arc::new(LocalSessionManager::default()),
            StreamableHttpServerConfig::default().disable_allowed_hosts(),
        );
        let gate = move |request: axum::extract::Request, next: axum::middleware::Next| {
            let required = required.clone();
            async move {
                if let Some((name, value)) = required {
                    let ok = request
                        .headers()
                        .get(name)
                        .and_then(|v| v.to_str().ok())
                        .is_some_and(|v| v == value);
                    if !ok {
                        return axum::http::StatusCode::UNAUTHORIZED.into_response();
                    }
                }
                next.run(request).await
            }
        };
        use axum::response::IntoResponse;
        let app = axum::Router::new()
            .nest_service("/mcp", service)
            .layer(axum::middleware::from_fn(gate));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        format!("http://{addr}/mcp")
    }

    pub(crate) fn mock(tools: &[&str]) -> MockUpstream {
        MockUpstream {
            tools: Arc::new(std::sync::Mutex::new(
                tools.iter().map(|t| t.to_string()).collect(),
            )),
            calls: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        }
    }

    fn target(url: &str, header: Option<(&str, &str)>) -> Target {
        let mut headers = HashMap::new();
        if let Some((name, value)) = header {
            headers.insert(
                axum::http::HeaderName::from_bytes(name.as_bytes()).unwrap(),
                axum::http::HeaderValue::from_str(value).unwrap(),
            );
        }
        Target {
            url: url.to_string(),
            headers,
        }
    }

    #[tokio::test]
    async fn a_probe_lists_the_upstreams_tools_with_its_credential() {
        let up = mock(&["search", "get_issue"]);
        let url = serve_mock(up.clone(), Some(("x-api-key", "k-right".into()))).await;
        let result = probe(target(&url, Some(("X-Api-Key", "k-right"))))
            .await
            .unwrap();
        assert_eq!(result.server_name, "mock-upstream");
        let mut names: Vec<String> = result.tools.iter().map(|t| t.name.to_string()).collect();
        names.sort();
        assert_eq!(names, vec!["get_issue", "search"]);
        // A probe lists; it never calls a tool.
        assert_eq!(up.calls.load(std::sync::atomic::Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn a_refused_credential_is_unauthorized_not_unavailable() {
        let url = serve_mock(mock(&["search"]), Some(("x-api-key", "k-right".into()))).await;
        let error = probe(target(&url, Some(("X-Api-Key", "k-wrong"))))
            .await
            .err()
            .unwrap();
        assert_eq!(error, UpstreamError::Unauthorized);
        assert!(error.needs_reauth());
    }

    #[tokio::test]
    async fn an_unreachable_upstream_is_unavailable() {
        // A port nothing listens on: bind one, then drop it.
        let port = {
            let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap().port()
        };
        let error = probe(target(&format!("http://127.0.0.1:{port}/mcp"), None))
            .await
            .err()
            .unwrap();
        assert_eq!(error, UpstreamError::Unavailable);
        assert!(!error.needs_reauth());
    }

    #[tokio::test]
    async fn a_redirect_is_not_followed_with_the_credential() {
        // An upstream that redirects to a second server: the credential must
        // not arrive there, because the client follows no redirect at all.
        let seen = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let seen_by_target = seen.clone();
        let target_app = axum::Router::new().fallback(move |request: axum::extract::Request| {
            let seen = seen_by_target.clone();
            async move {
                let header = request
                    .headers()
                    .get("x-api-key")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("")
                    .to_string();
                seen.lock().unwrap().push(header);
                axum::http::StatusCode::NOT_FOUND
            }
        });
        let l2 = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let elsewhere = format!("http://{}/steal", l2.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(l2, target_app).await.unwrap() });
        let redirecting = axum::Router::new().fallback(move || {
            let to = elsewhere.clone();
            async move {
                (
                    axum::http::StatusCode::TEMPORARY_REDIRECT,
                    [(axum::http::header::LOCATION, to)],
                )
            }
        });
        let l1 = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/mcp", l1.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(l1, redirecting).await.unwrap() });

        let error = probe(target(
            &url,
            Some(("X-Api-Key", "upstream-canary-redirect")),
        ))
        .await
        .err()
        .unwrap();
        assert_ne!(error, UpstreamError::Unauthorized);
        assert!(
            seen.lock().unwrap().is_empty(),
            "a redirect was followed: {:?}",
            seen.lock().unwrap()
        );
    }

    /// Network, so not in CI: `cargo test -- --ignored real_https`. A real
    /// upstream over real TLS, through `ring` and the bundled roots, with no
    /// process-wide provider installed. Notion answers an unauthenticated
    /// MCP request with 401 and a challenge (G1), which must read as
    /// `Unauthorized` — not as a TLS or protocol failure.
    #[tokio::test]
    #[ignore]
    async fn a_real_https_upstream_answers_through_ring_and_the_bundled_roots() {
        let error = probe(target("https://mcp.notion.com/mcp", None))
            .await
            .err()
            .unwrap();
        assert_eq!(error, UpstreamError::Unauthorized);
    }

    #[test]
    fn new_tools_default_off_and_known_decisions_stick() {
        let mut c = crate::gateway::connections::tests::connection();
        // First listing: all on.
        assert!(reconcile_tools(&mut c, &["b".into(), "a".into()]).is_empty());
        assert_eq!(c.tool_allowlist, vec!["a", "b"]);
        // The owner turns `b` off.
        c.tool_allowlist = vec!["a".into()];
        // A new tool appears: reported, and off.
        let new = reconcile_tools(&mut c, &["a".into(), "b".into(), "c".into()]);
        assert_eq!(new, vec!["c"]);
        assert_eq!(c.tool_allowlist, vec!["a"]);
        // `b` vanishes and returns: still the owner's decision, not "new".
        assert!(reconcile_tools(&mut c, &["a".into()]).is_empty());
        assert!(reconcile_tools(&mut c, &["a".into(), "b".into()]).is_empty());
        assert_eq!(c.tool_allowlist, vec!["a"]);
    }

    #[test]
    fn only_the_once_methods_send_a_request_upstream() {
        // AM26 / G3. The re-sending helpers, spelled so that this test's own
        // source does not match its own search.
        let forbidden: Vec<String> = [
            "call_tool",
            "call_tool_with_mrtr_max_rounds",
            "get_prompt",
            "get_prompt_with_mrtr_max_rounds",
            "read_resource",
            "read_resource_with_mrtr_max_rounds",
        ]
        .iter()
        .map(|m| format!(".{m}("))
        .collect();
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut files = vec![root.join("ops.rs")];
        for entry in std::fs::read_dir(root.join("gateway")).unwrap() {
            files.push(entry.unwrap().path());
        }
        let mut offences = Vec::new();
        for file in files {
            let text = std::fs::read_to_string(&file).unwrap();
            for (n, line) in text.lines().enumerate() {
                for f in &forbidden {
                    if line.contains(f.as_str()) {
                        offences.push(format!("{}:{}: {}", file.display(), n + 1, line.trim()));
                    }
                }
            }
        }
        assert!(
            offences.is_empty(),
            "a re-sending rmcp method is used; use the *_once form:\n{}",
            offences.join("\n")
        );
    }

    #[test]
    fn a_target_debug_names_headers_not_values() {
        let t = target(
            "https://x.example/mcp",
            Some(("Authorization", "Bearer ghp_secret")),
        );
        let printed = format!("{t:?}");
        assert!(!printed.contains("ghp_secret"), "{printed}");
        assert!(printed.contains("authorization"));
    }
}
