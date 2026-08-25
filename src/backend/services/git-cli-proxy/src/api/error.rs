//! Resource-scoped canonical errors for the proxy's `/v1` handlers.
//!
//! One GTS namespace covers every failure this API has: each one is about the
//! repository the request names. Envelopes come from `toolkit-canonical-errors`
//! (RFC 9457 `application/problem+json`) — no error type of ours crosses the
//! API boundary.

use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use toolkit_canonical_errors::{CanonicalError, Problem, resource_error};

use crate::engine::metrics;
use crate::engine::runner::GitError;
use crate::engine::store::StoreError;

use super::request::BadRequest;

#[resource_error("gts.cf.insight.git_cli_proxy.repository.v1~")]
pub struct RepositoryError;

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error(transparent)]
    BadRequest(#[from] BadRequest),
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error("git failed: {0}")]
    Git(#[from] GitError),
    #[error("the proxy token was missing or wrong")]
    ProxyTokenRejected,
    #[error("failed to serialize the response: {0}")]
    Serialization(String),
}

/// `413` has no canonical category: the catalogue maps `failed_precondition`
/// to `400`, and its only `429` category is the one the connector RETRIES. The
/// connector contract predates the catalogue and pins `413` as the permanent
/// "this repository is too big" signal, so the envelope stays canonical and
/// only the status is overridden.
const REPO_TOO_LARGE: StatusCode = StatusCode::PAYLOAD_TOO_LARGE;

/// §3.6: admission is refused when the cache is full and nothing can be
/// reclaimed. That clears as soon as a reader releases an entry, so the
/// caller is asked back rather than failed.
const ADMISSION_RETRY_AFTER_SECONDS: u64 = 30;

/// Origin asked this client to slow down. Longer than the cache's own hint:
/// a vendor rate-limit window outlives a reader releasing an entry.
const THROTTLED_RETRY_AFTER_SECONDS: u64 = 60;

/// A prefetch refused for space schedules a pressure purge as it rejects, so
/// "later" is one repack away — much sooner than a full admission cycle.
const PRESSURE_RETRY_AFTER_SECONDS: u64 = 5;

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        // The admission reasons are counted where the store decides them;
        // these two 429 causes reach the wire without touching admission and
        // would otherwise be invisible in the rejection counter.
        match &self {
            Self::Store(StoreError::Busy { .. }) => {
                metrics::record_rejection(metrics::RejectReason::PreparationWait);
            }
            Self::Store(StoreError::Throttled) | Self::Git(GitError::Throttled) => {
                metrics::record_rejection(metrics::RejectReason::OriginThrottled);
            }
            Self::Store(StoreError::OriginUnavailable) | Self::Git(GitError::OriginUnavailable) => {
                metrics::record_origin_unavailable();
            }
            _ => {}
        }

        let retry_after = self.retry_after();
        let status_override = self.status_override();
        let error = self.into_canonical();

        if status_override.is_none() && retry_after.is_none() {
            // The crate's own `IntoResponse` stashes the error in the response
            // extensions so the host middleware can log `diagnostic()` without
            // putting it on the wire. Prefer it whenever nothing needs doctoring.
            return error.into_response();
        }

        let mut problem = Problem::from(error);
        if let Some(status) = status_override {
            problem.status = status.as_u16();
        }

        let mut response = problem.into_response();
        if let Some(seconds) = retry_after
            && let Ok(value) = HeaderValue::from_str(&seconds.to_string())
        {
            // The crate deliberately keeps this hint in the body for
            // `resource_exhausted` (the header is reserved for
            // `service_unavailable`), but the connector's backoff strategy
            // reads the header, and that contract predates the catalogue.
            response.headers_mut().insert(header::RETRY_AFTER, value);
        }
        response
    }
}

impl ApiError {
    /// The status this error will carry on the wire. Exposed so a request can
    /// be recorded against its outcome without first consuming the error.
    #[must_use]
    pub fn status_code(&self) -> u16 {
        if let Some(status) = self.status_override() {
            return status.as_u16();
        }
        match self {
            Self::BadRequest(_) => StatusCode::BAD_REQUEST.as_u16(),
            Self::ProxyTokenRejected
            | Self::Store(StoreError::AuthRejected)
            | Self::Git(GitError::AuthRejected) => StatusCode::UNAUTHORIZED.as_u16(),
            Self::Store(StoreError::NotFound | StoreError::OriginUnavailable)
            | Self::Git(GitError::NotFound | GitError::OriginUnavailable) => {
                StatusCode::NOT_FOUND.as_u16()
            }
            Self::Store(StoreError::SnapshotChanged { .. }) => StatusCode::CONFLICT.as_u16(),
            Self::Store(StoreError::Busy { .. } | StoreError::Throttled)
            | Self::Git(
                GitError::AdmissionRejected | GitError::TransientlyOverCap | GitError::Throttled,
            ) => StatusCode::TOO_MANY_REQUESTS.as_u16(),
            _ => StatusCode::INTERNAL_SERVER_ERROR.as_u16(),
        }
    }

    fn status_override(&self) -> Option<StatusCode> {
        match self {
            Self::Store(StoreError::TooLarge { .. }) | Self::Git(GitError::TooLarge { .. }) => {
                Some(REPO_TOO_LARGE)
            }
            _ => None,
        }
    }

    fn retry_after(&self) -> Option<u64> {
        match self {
            Self::Store(StoreError::Busy { retry_after }) => Some(retry_after.as_secs()),
            Self::Git(GitError::AdmissionRejected) => Some(ADMISSION_RETRY_AFTER_SECONDS),
            Self::Git(GitError::TransientlyOverCap) => Some(PRESSURE_RETRY_AFTER_SECONDS),
            Self::Store(StoreError::Throttled) | Self::Git(GitError::Throttled) => {
                Some(THROTTLED_RETRY_AFTER_SECONDS)
            }
            _ => None,
        }
    }

    fn into_canonical(self) -> CanonicalError {
        match self {
            Self::BadRequest(e) => RepositoryError::invalid_argument()
                .with_field_violation(bad_request_field(&e), e.to_string(), "INVALID")
                .create(),

            // A reader hits the same origin failures as the clone: a partial
            // clone lazily fetches from the promisor remote, so any git step
            // can be rejected by the vendor. Classify by kind, never by which
            // layer raised it.
            Self::Store(StoreError::AuthRejected) | Self::Git(GitError::AuthRejected) => {
                CanonicalError::unauthenticated()
                    .with_reason("ORIGIN_CREDENTIALS_REJECTED")
                    .create()
            }
            // Distinct reason: the caller's own token, not the git credentials
            // it forwarded. Confusing the two sends an operator to the wrong
            // secret.
            Self::ProxyTokenRejected => CanonicalError::unauthenticated()
                .with_reason("PROXY_TOKEN_REJECTED")
                .create(),
            Self::Store(StoreError::NotFound) | Self::Git(GitError::NotFound) => {
                RepositoryError::not_found("repository not found at origin")
                    // The caller's own `repo` value is not echoed back: the
                    // envelope names the kind of thing, the detail says what
                    // happened, and the request already carries the URL.
                    .with_resource("repository")
                    .create()
            }
            // Suspended, disabled, or over the vendor's own limit at origin.
            // The same 404 the connector already skips a deleted repository
            // on, with a distinct detail so an operator can tell the two
            // apart in logs and problem bodies.
            Self::Store(StoreError::OriginUnavailable) | Self::Git(GitError::OriginUnavailable) => {
                RepositoryError::not_found("origin declines to serve the repository")
                    .with_resource("repository")
                    .create()
            }
            Self::Git(GitError::AdmissionRejected) => RepositoryError::resource_exhausted(
                "the cache is full and nothing can be reclaimed",
            )
            .with_quota_violation("cache_disk_budget", "no reclaimable entry")
            .with_quota_violation_retry_after_seconds(ADMISSION_RETRY_AFTER_SECONDS)
            .create(),
            // Retryable, unlike the 413 above: on the page-serve path the
            // measurement includes blob weight the scheduled purge reclaims.
            Self::Git(GitError::TransientlyOverCap) => RepositoryError::resource_exhausted(
                "the entry is over its cap until purged blobs are reclaimed",
            )
            .with_quota_violation("entry_blob_weight", "purge scheduled")
            .with_quota_violation_retry_after_seconds(PRESSURE_RETRY_AFTER_SECONDS)
            .create(),
            Self::Store(StoreError::Busy { retry_after }) => {
                RepositoryError::resource_exhausted("repository is being prepared")
                    .with_quota_violation("repository_preparation", "clone or fetch in progress")
                    .with_quota_violation_retry_after_seconds(retry_after.as_secs())
                    .create()
            }
            // The vendor's own limiter, not ours. Same shape as a cache
            // refusal so the connector's declarative handler backs off
            // identically, but a distinct quota subject so an operator can
            // tell "we are full" from "GitHub is throttling us".
            Self::Store(StoreError::Throttled) | Self::Git(GitError::Throttled) => {
                RepositoryError::resource_exhausted("origin is throttling this client")
                    .with_quota_violation("origin_rate_limit", "origin refused with a rate limit")
                    .with_quota_violation_retry_after_seconds(THROTTLED_RETRY_AFTER_SECONDS)
                    .create()
            }
            Self::Store(StoreError::SnapshotChanged { current }) => RepositoryError::aborted(
                format!("repository snapshot changed (current generation {current})"),
            )
            .with_reason("SNAPSHOT_CHANGED")
            .create(),
            // Permanent by design: retrying an oversized repository just burns
            // the budget again. The operator raises the cap or excludes it.
            Self::Store(StoreError::TooLarge { cap_bytes })
            | Self::Git(GitError::TooLarge { cap_bytes }) => RepositoryError::failed_precondition()
                .with_precondition_violation(
                    "REPOSITORY_SIZE_CAP",
                    "repository",
                    format!("repository exceeds the per-repository cap of {cap_bytes} bytes"),
                )
                .create(),

            Self::Store(
                internal @ (StoreError::Git(_) | StoreError::Io(_) | StoreError::PromisorRefused),
            ) => internal_error(&internal),
            Self::Git(internal) => internal_error(&internal),
            Self::Serialization(internal) => internal_error(&internal),
        }
    }
}

/// The answer for a handler that outlived its budget (`api::HANDLER_BUDGET`).
/// A plain 503 envelope: the connectors' declarative handlers RETRY 503, and
/// the detail deliberately names no internals — the log line next to it does.
#[must_use]
pub fn handler_timed_out() -> Response {
    CanonicalError::service_unavailable()
        .with_detail("the request outlived the handler budget")
        .create()
        .into_response()
}

/// Detail naming a path or a git invocation is logged here and never reaches
/// the wire; the crate additionally `serde(skip)`s an internal description.
fn internal_error(error: &dyn std::fmt::Display) -> CanonicalError {
    tracing::error!(error = %error, "request failed");
    CanonicalError::internal(error.to_string()).create()
}

/// The request part a [`BadRequest`] is about, so the envelope's field
/// violation points the caller at something actionable.
fn bad_request_field(error: &BadRequest) -> String {
    match error {
        BadRequest::MissingHeader(name)
        | BadRequest::NotANumber(name)
        | BadRequest::MissingParam(name) => (*name).to_owned(),
        BadRequest::MalformedToken => "page_token".to_owned(),
        BadRequest::MalformedSha(_) => "sha".to_owned(),
        BadRequest::BadRepoUrl(_) => "repo".to_owned(),
        BadRequest::MalformedQuery => "query".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    //! Wire-shape contract for the proxy's error responses.
    //!
    //! The connectors' declarative error handlers key on the STATUS and the
    //! `Retry-After` header — never on the body — so those are what these
    //! tests pin, alongside the RFC 9457 envelope the platform requires.

    use std::time::Duration;

    use axum::body::to_bytes;

    use super::*;

    async fn problem(error: ApiError) -> (StatusCode, Option<String>, serde_json::Value) {
        let response = error.into_response();
        let status = response.status();
        let retry_after = response
            .headers()
            .get(header::RETRY_AFTER)
            .and_then(|v| v.to_str().ok())
            .map(ToOwned::to_owned);
        let content_type = response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(ToOwned::to_owned);
        assert_eq!(
            content_type.as_deref(),
            Some("application/problem+json"),
            "every error is an RFC 9457 envelope"
        );

        let bytes = match to_bytes(response.into_body(), 1 << 20).await {
            Ok(bytes) => bytes,
            Err(e) => panic!("read body: {e}"),
        };
        let Ok(body) = serde_json::from_slice(&bytes) else {
            panic!("body must be JSON")
        };
        (status, retry_after, body)
    }

    #[tokio::test]
    async fn maps_every_failure_to_its_documented_status() {
        let cases: Vec<(ApiError, StatusCode)> = vec![
            (
                BadRequest::MissingHeader("x-tenant-id").into(),
                StatusCode::BAD_REQUEST,
            ),
            (StoreError::AuthRejected.into(), StatusCode::UNAUTHORIZED),
            (StoreError::NotFound.into(), StatusCode::NOT_FOUND),
            (
                StoreError::Busy {
                    retry_after: Duration::from_secs(30),
                }
                .into(),
                StatusCode::TOO_MANY_REQUESTS,
            ),
            (
                StoreError::SnapshotChanged { current: 5 }.into(),
                StatusCode::CONFLICT,
            ),
            (
                StoreError::TooLarge { cap_bytes: 1024 }.into(),
                StatusCode::PAYLOAD_TOO_LARGE,
            ),
            (
                StoreError::Git("boom".to_owned()).into(),
                StatusCode::INTERNAL_SERVER_ERROR,
            ),
            // A repository the origin refuses to serve: permanent for that
            // repository, so the connector must see the skippable 404.
            (StoreError::OriginUnavailable.into(), StatusCode::NOT_FOUND),
            // The same failures raised by a reader step, not by the clone.
            (GitError::AuthRejected.into(), StatusCode::UNAUTHORIZED),
            (GitError::NotFound.into(), StatusCode::NOT_FOUND),
            (GitError::OriginUnavailable.into(), StatusCode::NOT_FOUND),
            (
                GitError::TooLarge { cap_bytes: 1024 }.into(),
                StatusCode::PAYLOAD_TOO_LARGE,
            ),
            // Over-cap on the page-serve path is transient blob weight a
            // purge reclaims — retryable, never the permanent 413.
            (
                GitError::TransientlyOverCap.into(),
                StatusCode::TOO_MANY_REQUESTS,
            ),
            (
                GitError::Failed("boom".to_owned()).into(),
                StatusCode::INTERNAL_SERVER_ERROR,
            ),
            (
                GitError::PromisorRefused.into(),
                StatusCode::INTERNAL_SERVER_ERROR,
            ),
            // §3.6: the cache is full and nothing could be reclaimed.
            (
                GitError::AdmissionRejected.into(),
                StatusCode::TOO_MANY_REQUESTS,
            ),
        ];
        for (error, expected) in cases {
            let label = error.to_string();
            let (status, _, body) = problem(error).await;
            assert_eq!(status, expected, "for {label}");
            assert_eq!(body["status"], expected.as_u16(), "for {label}");
        }
    }

    #[tokio::test]
    async fn busy_carries_retry_after_and_others_do_not() {
        let (_, retry_after, body) = problem(
            StoreError::Busy {
                retry_after: Duration::from_secs(42),
            }
            .into(),
        )
        .await;
        assert_eq!(retry_after.as_deref(), Some("42"));
        assert_eq!(
            body["context"]["violations"][0]["retry_after_seconds"], 42,
            "the envelope carries the same hint as the header"
        );

        let (_, retry_after, _) = problem(StoreError::NotFound.into()).await;
        assert_eq!(retry_after, None, "only a preparing repo asks for a retry");

        let (status, retry_after, _) = problem(GitError::AdmissionRejected.into()).await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(
            retry_after.as_deref(),
            Some("30"),
            "a refused admission must tell the caller when to come back"
        );

        let (status, retry_after, _) = problem(GitError::TransientlyOverCap.into()).await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(
            retry_after.as_deref(),
            Some("5"),
            "a pressure rejection promises a purge-soon retry, not an admission cycle"
        );
    }

    #[tokio::test]
    async fn internal_errors_do_not_leak_paths_to_the_wire() {
        let (_, _, body) = problem(
            StoreError::Io(std::io::Error::other(
                "/var/lib/insight/repos/abc123 exploded",
            ))
            .into(),
        )
        .await;
        let rendered = body.to_string();
        assert!(
            !rendered.contains("/var/lib/insight"),
            "internal detail must stay in the log: {rendered}"
        );
    }

    #[tokio::test]
    async fn caller_actionable_errors_name_the_offending_field() {
        let (_, _, body) = problem(BadRequest::MissingHeader("x-git-token").into()).await;
        assert_eq!(
            body["context"]["field_violations"][0]["field"],
            "x-git-token"
        );

        let (_, _, body) =
            problem(BadRequest::BadRepoUrl(crate::engine::url::CloneUrlError::Empty).into()).await;
        assert_eq!(body["context"]["field_violations"][0]["field"], "repo");
    }

    #[tokio::test]
    async fn the_gts_resource_type_is_the_service_namespace() {
        let (_, _, body) = problem(StoreError::NotFound.into()).await;
        assert_eq!(
            body["context"]["resource_type"],
            "gts.cf.insight.git_cli_proxy.repository.v1~"
        );
    }
}
