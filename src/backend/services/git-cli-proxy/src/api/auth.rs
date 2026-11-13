//! Static bearer-token auth for the `/v1` routes.
//!
//! The service is cluster-internal: consumers are Airbyte connector pods, not
//! browser traffic through the platform gateway, so the host links no gateway
//! or authn gears and this middleware is the entire authentication surface. The expected token arrives via gear config (`proxy_token`,
//! provisioned by Helm); callers send `Authorization: Bearer <token>`.

use axum::extract::Request;
use axum::http::header;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use toolkit_security::crypto::constant_time_eq;

use super::error::ApiError;

#[derive(Clone)]
pub struct ProxyAuth {
    token: String,
}

impl ProxyAuth {
    /// Build the middleware state from the configured token. The config layer
    /// has already rejected an empty token at boot ([`crate::config`]).
    #[must_use]
    pub fn new(token: String) -> Self {
        Self { token }
    }

    /// Whether `presented` matches the configured token, in constant time
    /// (no early exit on the first differing byte).
    fn matches(&self, presented: &str) -> bool {
        constant_time_eq(self.token.as_bytes(), presented.as_bytes())
    }
}

/// Axum middleware: require `Authorization: Bearer <proxy_token>` on every
/// request it wraps. Rejections are a uniform `401` in the same RFC 9457
/// envelope as every other failure — the reason says which credential was
/// refused, and nothing more.
pub async fn require_bearer(
    axum::extract::State(auth): axum::extract::State<ProxyAuth>,
    request: Request,
    next: Next,
) -> Response {
    let presented = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));
    match presented {
        Some(token) if auth.matches(token) => next.run(request).await,
        _ => ApiError::ProxyTokenRejected.into_response(),
    }
}

#[cfg(test)]
mod tests {
    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::routing::get;
    use tower::ServiceExt;

    use super::*;

    fn app(token: &str) -> Router {
        let auth = ProxyAuth::new(token.to_owned());
        Router::new()
            .route("/v1/ping", get(|| async { "pong" }))
            .layer(axum::middleware::from_fn_with_state(auth, require_bearer))
    }

    async fn status_for(router: Router, auth_header: Option<&str>) -> StatusCode {
        let mut builder = Request::builder().uri("/v1/ping");
        if let Some(h) = auth_header {
            builder = builder.header("authorization", h);
        }
        let request = builder.body(Body::empty()).unwrap_or_default();
        match router.oneshot(request).await {
            Ok(response) => response.status(),
            Err(never) => match never {},
        }
    }

    #[tokio::test]
    async fn accepts_the_configured_token() {
        let status = status_for(app("s3cret"), Some("Bearer s3cret")).await;
        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn rejects_missing_wrong_and_malformed_credentials() {
        let cases: Vec<(&str, Option<&str>)> = vec![
            ("no header", None),
            ("wrong token", Some("Bearer nope")),
            ("wrong scheme", Some("Basic s3cret")),
            ("empty bearer", Some("Bearer ")),
            ("prefix of the token", Some("Bearer s3cre")),
            ("token with suffix", Some("Bearer s3cret2")),
            ("same length, one byte off", Some("Bearer s3creT")),
        ];
        for (name, header_value) in cases {
            let status = status_for(app("s3cret"), header_value).await;
            assert_eq!(status, StatusCode::UNAUTHORIZED, "case: {name}");
        }
    }
}
