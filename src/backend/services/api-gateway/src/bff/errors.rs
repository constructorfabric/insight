//! BFF errors, rendered through the constructor-fabric canonical error
//! envelope (`modkit-canonical-errors`).
//!
//! Internal callers pattern-match on `BffError` variants. The wire shape
//! is the same RFC 9457 `application/problem+json` envelope the rest of
//! the Insight backend emits — `type` is a `gts://…/cf.core.err.*` URI,
//! `context.resource_type` scopes BFF-owned resources, and structured
//! `field_violations` / `quota_violations` carry the failure reason.
//!
//! Mapping (`BffError` → canonical):
//!
//! | Variant            | Canonical                  | HTTP |
//! |--------------------|----------------------------|------|
//! | `Unauthorized`     | `unauthenticated` + reason | 401  |
//! | `BadRequest`       | `invalid_argument`         | 400  |
//! | `Idp`              | `service_unavailable`      | 503  |
//! | `StoreUnavailable` | `service_unavailable`      | 503  |
//! | `Internal`         | `internal` + diagnostic    | 500  |
//! | `RateLimited`      | `resource_exhausted`       | 429  |
//!
//! `Idp` used to map to HTTP 502, but the canonical-error standard set
//! has no 502 slot. 503 is the closest semantic match (external IdP
//! unreachable from the BFF's perspective) and keeps the wire envelope
//! consistent with the rest of the platform.

use axum::response::{IntoResponse, Response};
use modkit_canonical_errors::{CanonicalError, resource_error};
use thiserror::Error;

/// Resource namespace for every BFF-emitted canonical error. Same scheme
/// as the analytics-api namespaces — `gts.cf.<repo>.<service>.<area>.v1~`.
#[resource_error("gts.cf.insight.api_gateway.bff.v1~")]
pub struct BffResource;

#[derive(Debug, Error)]
#[allow(dead_code)]
pub enum BffError {
    /// 401 — missing/expired/invalid session cookie, or OIDC state/nonce
    /// mismatch. The static `&str` is the wire-facing reason.
    #[error("unauthorized: {0}")]
    Unauthorized(&'static str),

    /// 400 — malformed request shape that wasn't caught by extractor
    /// validation. The static `&str` becomes a field-violation `reason`
    /// scoped to the synthetic `request` field.
    #[error("bad request: {0}")]
    BadRequest(&'static str),

    /// 503 — IdP unreachable or returned a non-OK status. The diagnostic
    /// string is logged server-side but never echoed in the wire body.
    #[error("upstream IdP error: {0}")]
    Idp(String),

    /// 503 — Redis unreachable (DD-BFF-06: fail closed). Diagnostic
    /// logged server-side only.
    #[error("session store unavailable: {0}")]
    StoreUnavailable(String),

    /// 500 — uncategorized internal failure. Diagnostic surfaces through
    /// `CanonicalError::diagnostic()` for server-side logs but stays out
    /// of the response body.
    #[error("internal: {0}")]
    Internal(#[from] anyhow::Error),

    /// 429 — login-state cap or per-IP rate limit hit.
    #[error("rate limited")]
    RateLimited,
}

impl BffError {
    fn to_canonical(&self) -> CanonicalError {
        match self {
            Self::Unauthorized(reason) => CanonicalError::unauthenticated()
                .with_reason((*reason).to_owned())
                .create(),
            Self::BadRequest(reason) => BffResource::invalid_argument()
                .with_field_violation("request", (*reason).to_owned(), "INVALID")
                .create(),
            // `service_unavailable` collapses both "IdP unreachable" and
            // "Redis unreachable" — wire-side they both mean "an upstream
            // the BFF depends on is currently down; try again later".
            // The variant-specific diagnostic is captured in the trace
            // log emitted by `into_response`.
            Self::Idp(_) | Self::StoreUnavailable(_) => {
                CanonicalError::service_unavailable().create()
            }
            Self::Internal(err) => CanonicalError::internal(err.to_string()).create(),
            Self::RateLimited => BffResource::resource_exhausted("rate limit exceeded on /auth/*")
                .with_quota_violation("auth_rate", "per-IP / per-pod auth rate limit hit")
                .create(),
        }
    }
}

impl IntoResponse for BffError {
    fn into_response(self) -> Response {
        // Log the full error chain before erasing it. The canonical
        // envelope intentionally hides diagnostic text from clients for
        // `Internal` / `ServiceUnavailable`; we keep that visible
        // server-side for operators correlating with `trace_id`.
        match &self {
            Self::Internal(_) => {
                tracing::error!(error = %self, "BFF internal error");
            }
            Self::Idp(_) | Self::StoreUnavailable(_) => {
                tracing::warn!(error = %self, "BFF upstream error");
            }
            _ => {
                tracing::warn!(error = %self, "BFF error");
            }
        }
        self.to_canonical().into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    use axum::http::header::CONTENT_TYPE;

    async fn problem_body(err: BffError) -> (u16, serde_json::Value) {
        let resp = err.into_response();
        let status = resp.status().as_u16();
        let ct = resp
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .expect("content-type present")
            .to_owned();
        assert_eq!(
            ct, "application/problem+json",
            "RFC 9457 mandates problem+json",
        );
        let bytes = to_bytes(resp.into_body(), 8192).await.expect("body");
        (
            status,
            serde_json::from_slice(&bytes).expect("problem json"),
        )
    }

    #[tokio::test]
    async fn unauthorized_maps_to_canonical_unauthenticated() {
        let (status, p) = problem_body(BffError::Unauthorized("no session")).await;
        assert_eq!(status, 401);
        assert_eq!(
            p["type"],
            "gts://gts.cf.core.errors.err.v1~cf.core.err.unauthenticated.v1~",
        );
        assert_eq!(p["title"], "Unauthenticated");
        assert_eq!(p["context"]["reason"], "no session");
    }

    #[tokio::test]
    async fn bad_request_maps_to_invalid_argument_with_field_violation() {
        let (status, p) = problem_body(BffError::BadRequest("state mismatch or expired")).await;
        assert_eq!(status, 400);
        assert_eq!(
            p["type"],
            "gts://gts.cf.core.errors.err.v1~cf.core.err.invalid_argument.v1~",
        );
        assert_eq!(
            p["context"]["resource_type"],
            "gts.cf.insight.api_gateway.bff.v1~",
        );
        let violations = p["context"]["field_violations"]
            .as_array()
            .expect("field violations present");
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0]["field"], "request");
        assert_eq!(violations[0]["description"], "state mismatch or expired");
        assert_eq!(violations[0]["reason"], "INVALID");
    }

    #[tokio::test]
    async fn idp_maps_to_service_unavailable() {
        let (status, p) = problem_body(BffError::Idp("token endpoint: timeout".into())).await;
        assert_eq!(status, 503);
        assert_eq!(
            p["type"],
            "gts://gts.cf.core.errors.err.v1~cf.core.err.service_unavailable.v1~",
        );
        // Diagnostic stays in tracing logs; never in the wire body.
        let body_str = p.to_string();
        assert!(
            !body_str.contains("token endpoint: timeout"),
            "diagnostic must not leak to the wire response",
        );
    }

    #[tokio::test]
    async fn store_unavailable_maps_to_service_unavailable() {
        let (status, p) = problem_body(BffError::StoreUnavailable(
            "redis connection refused".into(),
        ))
        .await;
        assert_eq!(status, 503);
        assert_eq!(
            p["type"],
            "gts://gts.cf.core.errors.err.v1~cf.core.err.service_unavailable.v1~",
        );
        let body_str = p.to_string();
        assert!(
            !body_str.contains("redis connection refused"),
            "diagnostic must not leak to the wire response",
        );
    }

    #[tokio::test]
    async fn internal_renders_as_canonical_internal_without_leaking_detail() {
        let (status, p) = problem_body(BffError::Internal(anyhow::anyhow!("secret-detail"))).await;
        assert_eq!(status, 500);
        assert_eq!(
            p["type"],
            "gts://gts.cf.core.errors.err.v1~cf.core.err.internal.v1~",
        );
        let body_str = p.to_string();
        assert!(
            !body_str.contains("secret-detail"),
            "anyhow detail must not leak to the wire response",
        );
    }

    #[tokio::test]
    async fn rate_limited_maps_to_resource_exhausted_with_quota_violation() {
        let (status, p) = problem_body(BffError::RateLimited).await;
        assert_eq!(status, 429);
        assert_eq!(
            p["type"],
            "gts://gts.cf.core.errors.err.v1~cf.core.err.resource_exhausted.v1~",
        );
        assert_eq!(
            p["context"]["resource_type"],
            "gts.cf.insight.api_gateway.bff.v1~",
        );
        // The canonical envelope flattens quota-category violations under
        // `context.violations`. Pin both the subject (machine-readable
        // throttle id) and the description.
        let violations = p["context"]["violations"]
            .as_array()
            .expect("quota violations present");
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0]["subject"], "auth_rate");
        assert_eq!(
            violations[0]["description"],
            "per-IP / per-pod auth rate limit hit",
        );
    }
}
