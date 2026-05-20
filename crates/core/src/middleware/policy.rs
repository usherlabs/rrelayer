use std::net::IpAddr;

use axum::async_trait;
use axum::extract::{ConnectInfo, FromRequestParts, Request};
use axum::http::{request::Parts, HeaderMap, StatusCode};
use axum::middleware::Next;
use axum::response::Response;

/// Per-request context populated by `inject_policy_context` and consumed by
/// per-handler policy validation in `AppState::validate_request_policy`.
#[derive(Debug, Clone)]
pub struct PolicyContext {
    /// Resolved client IP. `None` when the peer info is not available (e.g.
    /// service-to-service tests). When `trust_forwarded_for` is enabled in
    /// the server config, the leftmost entry of `x-forwarded-for` wins.
    pub client_ip: Option<IpAddr>,
}

impl PolicyContext {
    pub fn empty() -> Self {
        Self { client_ip: None }
    }
}

fn extract_client_ip(
    headers: &HeaderMap,
    connect_ip: Option<IpAddr>,
    trust_xff: bool,
) -> Option<IpAddr> {
    if trust_xff {
        if let Some(raw) = headers.get("x-forwarded-for") {
            return raw
                .to_str()
                .ok()
                .and_then(|value| value.split(',').next())
                .and_then(|first| first.trim().parse::<IpAddr>().ok());
        }
    }

    connect_ip
}

/// Middleware that:
/// 1. Resolves the client IP, honoring `x-forwarded-for` only when the
///    server is configured with `trust_forwarded_for: true`.
/// 2. Inserts `PolicyContext` into request extensions for handlers and the
///    `AppState::validate_request_policy` helper.
pub async fn inject_policy_context(
    trust_xff: bool,
    req: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    let (mut parts, body) = req.into_parts();

    let connect_ip =
        parts.extensions.get::<ConnectInfo<std::net::SocketAddr>>().map(|info| info.0.ip());
    let client_ip = extract_client_ip(&parts.headers, connect_ip, trust_xff);

    let ctx = PolicyContext { client_ip };
    parts.extensions.insert(ctx);

    let req = Request::from_parts(parts, body);
    Ok(next.run(req).await)
}

#[async_trait]
impl<S> FromRequestParts<S> for PolicyContext
where
    S: Send + Sync,
{
    type Rejection = StatusCode;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        parts.extensions.get::<PolicyContext>().cloned().ok_or(StatusCode::INTERNAL_SERVER_ERROR)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    #[test]
    fn trust_xff_returns_first_entry() {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", HeaderValue::from_static("203.0.113.7, 10.0.0.1"));
        let connect = Some("10.0.0.1".parse().unwrap());
        let ip = extract_client_ip(&headers, connect, true).unwrap();
        assert_eq!(ip.to_string(), "203.0.113.7");
    }

    #[test]
    fn no_trust_xff_falls_back_to_connect_info() {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", HeaderValue::from_static("203.0.113.7, 10.0.0.1"));
        let connect = Some("10.0.0.1".parse().unwrap());
        let ip = extract_client_ip(&headers, connect, false).unwrap();
        assert_eq!(ip.to_string(), "10.0.0.1");
    }

    #[test]
    fn trust_xff_with_malformed_header_returns_none() {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", HeaderValue::from_static("not-an-ip"));
        let connect = Some("10.0.0.1".parse().unwrap());
        let ip = extract_client_ip(&headers, connect, true);
        assert!(ip.is_none());
    }

    #[test]
    fn trust_xff_without_header_falls_back_to_connect_info() {
        let headers = HeaderMap::new();
        let connect = Some("10.0.0.1".parse().unwrap());
        let ip = extract_client_ip(&headers, connect, true).unwrap();
        assert_eq!(ip.to_string(), "10.0.0.1");
    }

    #[test]
    fn policy_context_empty_has_no_client_ip() {
        let ctx = PolicyContext::empty();
        assert!(ctx.client_ip.is_none());
    }
}
