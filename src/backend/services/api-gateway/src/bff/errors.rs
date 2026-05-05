//! Typed errors for the BFF, with RFC 9457 Problem conversion.
//!
//! Public-facing handler errors all flow through the `IntoResponse` impl on
//! `BffError`, which renders a `application/problem+json` body. Internal
//! callers can pattern-match on the variant.

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;
use thiserror::Error;

#[derive(Debug, Error)]
#[allow(dead_code)]
pub enum BffError {
    /// Generic 401 — used for missing/expired/invalid session cookies and
    /// for OIDC state/nonce mismatches.
    #[error("unauthorized: {0}")]
    Unauthorized(&'static str),

    /// 400 — malformed request shape that wasn't caught by extractor
    /// validation.
    #[error("bad request: {0}")]
    BadRequest(&'static str),

    /// 502 — IdP unreachable or returned a non-OK status.
    #[error("upstream IdP error: {0}")]
    Idp(String),

    /// 503 — Redis unreachable (DD-BFF-06: fail closed).
    #[error("session store unavailable: {0}")]
    StoreUnavailable(String),

    /// 500 — anything we did not categorize. Detail is intentionally
    /// generic in the response body; the source error is logged.
    #[error("internal: {0}")]
    Internal(#[from] anyhow::Error),

    /// 429 — login-state cap or per-IP rate limit hit.
    #[error("rate limited")]
    RateLimited,
}

impl BffError {
    fn problem_parts(&self) -> (StatusCode, &'static str, &'static str, String) {
        match self {
            Self::Unauthorized(msg) => (
                StatusCode::UNAUTHORIZED,
                "Unauthorized",
                "urn:insight:error:unauthorized",
                (*msg).to_string(),
            ),
            Self::BadRequest(msg) => (
                StatusCode::BAD_REQUEST,
                "Bad Request",
                "urn:insight:error:bad_request",
                (*msg).to_string(),
            ),
            Self::Idp(detail) => (
                StatusCode::BAD_GATEWAY,
                "Bad Gateway",
                "urn:insight:error:idp_unreachable",
                detail.clone(),
            ),
            Self::StoreUnavailable(detail) => (
                StatusCode::SERVICE_UNAVAILABLE,
                "Service Unavailable",
                "urn:insight:error:session_store_unavailable",
                detail.clone(),
            ),
            Self::Internal(_) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal Server Error",
                "urn:insight:error:internal",
                "internal error".to_owned(),
            ),
            Self::RateLimited => (
                StatusCode::TOO_MANY_REQUESTS,
                "Too Many Requests",
                "urn:insight:error:rate_limited",
                "rate limited".to_owned(),
            ),
        }
    }
}

impl IntoResponse for BffError {
    fn into_response(self) -> Response {
        let (status, title, type_uri, detail) = self.problem_parts();

        // Log the full error chain for Internal errors — never put it in
        // the body.
        if matches!(self, Self::Internal(_)) {
            tracing::error!(error = %self, "BFF internal error");
        } else {
            tracing::warn!(error = %self, status = %status.as_u16(), "BFF error");
        }

        let body = json!({
            "type": type_uri,
            "title": title,
            "status": status.as_u16(),
            "detail": detail,
        });

        let mut resp = (status, Json(body)).into_response();
        // RFC 9457 §3 mandates application/problem+json
        resp.headers_mut().insert(
            axum::http::header::CONTENT_TYPE,
            axum::http::HeaderValue::from_static("application/problem+json"),
        );
        resp
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;

    #[tokio::test]
    async fn unauthorized_renders_problem_json() {
        let r = BffError::Unauthorized("no session").into_response();
        assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
        let ct = r
            .headers()
            .get(axum::http::header::CONTENT_TYPE)
            .expect("ct");
        assert_eq!(ct, "application/problem+json");
        let bytes = to_bytes(r.into_body(), 4096).await.expect("body");
        let v: serde_json::Value = serde_json::from_slice(&bytes).expect("json");
        assert_eq!(v["status"], 401);
        assert_eq!(v["type"], "urn:insight:error:unauthorized");
    }

    #[tokio::test]
    async fn internal_response_does_not_leak_detail() {
        let r = BffError::Internal(anyhow::anyhow!("secret-implementation-detail")).into_response();
        let bytes = to_bytes(r.into_body(), 4096).await.expect("body");
        let v: serde_json::Value = serde_json::from_slice(&bytes).expect("json");
        assert_eq!(v["detail"], "internal error");
    }
}
