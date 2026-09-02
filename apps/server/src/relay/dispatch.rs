//! Turning a tunnelled `HTTP_REQUEST_HEAD` back into an `http::Request` and
//! handing it to the router.
//!
//! **The relay adds no server-side logic path (R13).** A tunnelled request is
//! a second way for a `Request` to arrive at the same `Service`, so it runs
//! through the same `require_auth`, the same tier routers, the same `ops.rs`
//! calls and the same error mapping as a LAN request. There is no loopback TCP
//! hop and no bypass: if a handler cannot serve a relayed request, the handler
//! is what changes.
//!
//! Everything specific to the tunnel happens *here*, in the one pass that
//! builds the request — before the router sees it.

use axum::body::Body;
use axum::http::{HeaderName, HeaderValue, Request, StatusCode};

use super::proto::RequestHead;

// Headers a client must never be able to set on a relayed request.
//
// `HTTP_REQUEST_HEAD.headers` is client-supplied end to end: the relay
// forwards it verbatim (§5.2 forbids rewriting the proxied exchange), so
// whatever a client puts in header space arrives here. These four are treated
// elsewhere in the server as *proxy-set* facts — `web_bootstrap_nonce` refuses
// to mint a nonce when one is present, on the reasoning that the peer address
// belongs to a proxy and binding to it would bind the whole LAN.
//
// A client that could set them on every relayed request would turn that
// refusal into a remote off-switch for web bootstrap. Client input must never
// reach header space other code treats as proxy-set, so it is stripped here
// rather than trusted anywhere downstream.
use crate::api::FORWARDING_HEADERS;

/// Reconstructs a tunnelled request.
///
/// `host` is the server's configured bind address (`host:port`), not the
/// relay's — see [`Dispatcher::host`].
pub fn build_request(
    head: &RequestHead,
    host: &HeaderValue,
    body: Body,
) -> Result<Request<Body>, BadHead> {
    let method = axum::http::Method::from_bytes(head.method.as_bytes()).map_err(|_| BadHead)?;
    // The path carries any query string; `Uri` parses both.
    let uri: axum::http::Uri = head.path.parse().map_err(|_| BadHead)?;
    if uri.scheme().is_some() || uri.authority().is_some() {
        // An absolute-form request-target would let a caller name a host the
        // `Host` rewrite below then contradicts. Origin-form only.
        return Err(BadHead);
    }

    let mut request = Request::builder()
        .method(method)
        .uri(uri)
        .body(body)
        .map_err(|_| BadHead)?;

    let headers = request.headers_mut();
    for (name, value) in head.headers.iter() {
        let lower = name.to_ascii_lowercase();
        if FORWARDING_HEADERS.contains(&lower.as_str()) {
            continue;
        }
        // `Host` is set below, from configuration. A client-supplied one is
        // dropped rather than appended, or the request would carry two.
        if lower == "host" {
            continue;
        }
        // **The body's framing is ours now, not the wire's.** The body arrives
        // as `0x01` frames and is rebuilt as a stream, so a `Content-Length`
        // from the original request describes bytes this `Request` does not
        // have — a handler that trusted it would wait for a remainder that is
        // never coming. `Transfer-Encoding` goes for the same reason.
        if lower == "content-length" || lower == "transfer-encoding" {
            continue;
        }
        let (Ok(name), Ok(value)) = (
            HeaderName::from_bytes(lower.as_bytes()),
            HeaderValue::from_str(value),
        ) else {
            // A header that cannot be represented is dropped, not fatal: one
            // malformed field should not fail a request the router would
            // otherwise serve.
            continue;
        };
        headers.append(name, value);
    }

    // **The `Host` rewrite, and why it is not cosmetic.** `mcp::allowed_hosts`
    // is rmcp's DNS-rebinding protection, and it is a list of the addresses
    // *this server* binds. A tunnelled request's `Host` is the relay's
    // hostname, which is never on that list, so every relayed MCP call would
    // be refused with an error naming nothing about the real cause.
    //
    // It is moot on the shipped deployment only by luck: `allowed_hosts`
    // returns an empty Vec for `0.0.0.0`, which rmcp reads as allow-all, and
    // production binds `0.0.0.0`. Anyone binding a concrete address would find
    // MCP-over-relay broken with no way to see why.
    headers.insert(axum::http::header::HOST, host.clone());

    // **`relay_peer_ip` is the caller's identity, and it arrives out of band.**
    // It is a field on `HTTP_REQUEST_HEAD` rather than a header precisely so a
    // client cannot forge it — the relay overwrites it unconditionally from
    // the socket it can see (§5.2), and the forwarding headers that *are*
    // forgeable were stripped above.
    //
    // `ConnectInfo` is the existing seam: `MaybePeer` reads it, and the login
    // limiter turns it into `CallerKey::Ip`. Putting it here means per-caller
    // rate limiting works over the tunnel with no handler changed and no
    // invented header. A peer that is absent or unparseable leaves no
    // `ConnectInfo`, which is `CallerKey::Unattributed` — a bounded shared
    // bucket, never unlimited (§5.2).
    if let Some(ip) = head.relay_peer_ip.as_deref().and_then(parse_peer_ip) {
        // Port 0: the relay attests an address, not a port, and nothing reads
        // the port. Inventing one would look like information it is not.
        request
            .extensions_mut()
            .insert(axum::extract::ConnectInfo(std::net::SocketAddr::new(ip, 0)));
    }

    Ok(request)
}

fn parse_peer_ip(raw: &str) -> Option<std::net::IpAddr> {
    let trimmed = raw.trim();
    if let Ok(ip) = trimmed.parse::<std::net::IpAddr>() {
        return Some(ip);
    }
    // A bracketed IPv6 literal, in case the relay took the address from a
    // socket string rather than from the address itself.
    trimmed
        .strip_prefix('[')
        .and_then(|r| r.strip_suffix(']'))
        .and_then(|inner| inner.parse().ok())
}

/// A head this server will not build a request from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BadHead;

impl BadHead {
    /// What the caller is told. A malformed head is the caller's fault, and
    /// the tunnel answers it the way the router would.
    pub fn status(self) -> StatusCode {
        StatusCode::BAD_REQUEST
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::relay::proto::WireHeaders;

    fn head(headers: Vec<(&str, &str)>, peer: Option<&str>) -> RequestHead {
        RequestHead {
            stream_id: 1,
            method: "GET".into(),
            path: "/v1/health".into(),
            headers: WireHeaders(
                headers
                    .into_iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect(),
            ),
            relay_peer_ip: peer.map(str::to_string),
        }
    }

    fn bind_host() -> HeaderValue {
        HeaderValue::from_static("127.0.0.1:8484")
    }

    #[test]
    fn host_is_rewritten_to_the_bind_address() {
        // The relay's hostname is never in `mcp::allowed_hosts`, so leaving it
        // would fail every relayed MCP call with an unrelated-looking error.
        let head = head(vec![("host", "relay.example.com")], None);
        let request = build_request(&head, &bind_host(), Body::empty()).unwrap();
        assert_eq!(
            request.headers()[axum::http::header::HOST],
            "127.0.0.1:8484"
        );
        assert_eq!(
            request
                .headers()
                .get_all(axum::http::header::HOST)
                .iter()
                .count(),
            1,
            "the client's Host is replaced, not appended to"
        );
    }

    #[test]
    fn a_request_with_no_host_still_gets_one() {
        let request = build_request(&head(vec![], None), &bind_host(), Body::empty()).unwrap();
        assert_eq!(
            request.headers()[axum::http::header::HOST],
            "127.0.0.1:8484"
        );
    }

    #[test]
    fn every_forwarding_header_is_stripped() {
        // Client-supplied end to end. Left in place, a client could switch off
        // web bootstrap for every relayed request by setting one.
        let head = head(
            vec![
                ("x-forwarded-for", "1.2.3.4"),
                ("x-forwarded-host", "evil.example"),
                ("x-real-ip", "1.2.3.4"),
                ("forwarded", "for=1.2.3.4"),
                ("accept", "application/json"),
            ],
            None,
        );
        let request = build_request(&head, &bind_host(), Body::empty()).unwrap();
        for stripped in FORWARDING_HEADERS {
            assert!(
                !request.headers().contains_key(stripped),
                "{stripped} reached the router"
            );
        }
        assert_eq!(
            request.headers()["accept"],
            "application/json",
            "an ordinary header still arrives"
        );
    }

    #[test]
    fn a_forwarding_header_is_stripped_whatever_its_case() {
        // `HeaderName` lowercases on parse, but the match happens before that,
        // so an uppercase spelling has to be folded first or it slips through.
        let head = head(vec![("X-Forwarded-For", "1.2.3.4")], None);
        let request = build_request(&head, &bind_host(), Body::empty()).unwrap();
        assert!(!request.headers().contains_key("x-forwarded-for"));
    }

    #[test]
    fn the_peer_ip_becomes_the_requests_connect_info() {
        // This is what makes `CallerKey::Ip` work over the tunnel — the login
        // limiter reads `ConnectInfo` through `MaybePeer`.
        let head = head(vec![], Some("203.0.113.7"));
        let request = build_request(&head, &bind_host(), Body::empty()).unwrap();
        let peer = request
            .extensions()
            .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
            .expect("a relayed request carries its attested peer");
        assert_eq!(peer.0.ip().to_string(), "203.0.113.7");
    }

    #[test]
    fn an_ipv6_peer_is_understood_bare_or_bracketed() {
        for raw in ["2001:db8::1", "[2001:db8::1]"] {
            let request =
                build_request(&head(vec![], Some(raw)), &bind_host(), Body::empty()).unwrap();
            let peer = request
                .extensions()
                .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
                .unwrap_or_else(|| panic!("{raw} should parse"));
            assert_eq!(peer.0.ip().to_string(), "2001:db8::1");
        }
    }

    #[test]
    fn a_missing_or_junk_peer_leaves_the_request_unattributed() {
        // Unattributed is a bounded shared bucket, not an unlimited one — so
        // the absence of an attested peer must not look like a valid address.
        for peer in [None, Some("not-an-ip"), Some("")] {
            let request = build_request(&head(vec![], peer), &bind_host(), Body::empty()).unwrap();
            assert!(
                request
                    .extensions()
                    .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
                    .is_none(),
                "{peer:?} must not become a peer"
            );
        }
    }

    #[test]
    fn a_client_cannot_forge_a_peer_through_a_header() {
        // The forwarding headers are stripped and `relay_peer_ip` is the only
        // input, so a header naming a different address changes nothing.
        let head = head(vec![("x-forwarded-for", "9.9.9.9")], Some("203.0.113.7"));
        let request = build_request(&head, &bind_host(), Body::empty()).unwrap();
        let peer = request
            .extensions()
            .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
            .unwrap();
        assert_eq!(peer.0.ip().to_string(), "203.0.113.7");
        assert!(!request.headers().contains_key("x-forwarded-for"));
    }

    #[test]
    fn the_wires_body_framing_headers_do_not_survive() {
        // The body is rebuilt from `0x01` frames, so a stale `Content-Length`
        // would describe bytes this request does not carry.
        let head = head(
            vec![("content-length", "9999"), ("transfer-encoding", "chunked")],
            None,
        );
        let request = build_request(&head, &bind_host(), Body::empty()).unwrap();
        assert!(!request.headers().contains_key("content-length"));
        assert!(!request.headers().contains_key("transfer-encoding"));
    }

    #[test]
    fn the_path_keeps_its_query_string() {
        let mut h = head(vec![], None);
        h.path = "/v1/notes?vault=abc&limit=10".into();
        let request = build_request(&h, &bind_host(), Body::empty()).unwrap();
        assert_eq!(request.uri().path(), "/v1/notes");
        assert_eq!(request.uri().query(), Some("vault=abc&limit=10"));
    }

    #[test]
    fn an_absolute_target_is_refused() {
        // Otherwise the request would name a host the `Host` rewrite then
        // contradicts, and which of the two wins is per-handler.
        let mut h = head(vec![], None);
        h.path = "http://evil.example/v1/health".into();
        assert!(build_request(&h, &bind_host(), Body::empty()).is_err());
    }

    #[test]
    fn a_malformed_method_or_path_is_refused() {
        let mut h = head(vec![], None);
        h.method = "BAD METHOD".into();
        assert!(build_request(&h, &bind_host(), Body::empty()).is_err());

        let mut h = head(vec![], None);
        h.path = "not a uri".into();
        assert!(build_request(&h, &bind_host(), Body::empty()).is_err());
    }

    #[test]
    fn the_method_survives() {
        let mut h = head(vec![], None);
        h.method = "DELETE".into();
        let request = build_request(&h, &bind_host(), Body::empty()).unwrap();
        assert_eq!(request.method(), axum::http::Method::DELETE);
    }
}
