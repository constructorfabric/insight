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
    #[error("every page-serve slot is taken; retry shortly")]
    ServeSaturated,
    #[error("failed to serialize the response: {0}")]
    Serialization(String),
}

const REPO_TOO_LARGE: StatusCode = StatusCode::PAYLOAD_TOO_LARGE;

const ADMISSION_RETRY_AFTER_SECONDS: u64 = 30;

const THROTTLED_RETRY_AFTER_SECONDS: u64 = 60;
const SERVE_RETRY_AFTER_SECONDS: u64 = 30;

const PRESSURE_RETRY_AFTER_SECONDS: u64 = 5;

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        match &self {
            Self::Store(StoreError::Busy { .. }) => {
                metrics::record_rejection(metrics::RejectReason::PreparationWait);
            }
            Self::ServeSaturated => {
                metrics::record_rejection(metrics::RejectReason::ServeSaturated);
            }
            Self::Store(StoreError::Throttled) | Self::Git(GitError::Throttled) => {
                metrics::record_rejection(metrics::RejectReason::OriginThrottled);
            }
            Self::Store(StoreError::OriginUnavailable) | Self::Git(GitError::OriginUnavailable) => {
                metrics::record_origin_unavailable();
            }
            Self::BadRequest(_)
            | Self::Store(_)
            | Self::Git(_)
            | Self::ProxyTokenRejected
            | Self::Serialization(_) => {}
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
            problem.status = Some(status.as_u16());
        }

        let mut response = problem.into_response();
        if let Some(seconds) = retry_after
            && let Ok(value) = HeaderValue::from_str(&seconds.to_string())
        {
            // The crate deliberately keeps this hint in the body for
            // `resource_exhausted` (the header is reserved for
            // `service_unavailable`), but the connector's backoff strategy
            // reads the header.
            response.headers_mut().insert(header::RETRY_AFTER, value);
        }
        response
    }
}

impl ApiError {
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
            Self::ServeSaturated
            | Self::Store(StoreError::Busy { .. } | StoreError::Throttled)
            | Self::Git(
                GitError::AdmissionRejected | GitError::TransientlyOverCap | GitError::Throttled,
            ) => StatusCode::TOO_MANY_REQUESTS.as_u16(),
            Self::Store(_) | Self::Git(_) | Self::Serialization(_) => {
                StatusCode::INTERNAL_SERVER_ERROR.as_u16()
            }
        }
    }

    fn status_override(&self) -> Option<StatusCode> {
        match self {
            Self::Store(StoreError::TooLarge { .. }) | Self::Git(GitError::TooLarge { .. }) => {
                Some(REPO_TOO_LARGE)
            }
            Self::BadRequest(_)
            | Self::Store(_)
            | Self::Git(_)
            | Self::ProxyTokenRejected
            | Self::ServeSaturated
            | Self::Serialization(_) => None,
        }
    }

    fn retry_after(&self) -> Option<u64> {
        match self {
            Self::Store(StoreError::Busy { retry_after }) => Some(retry_after.as_secs()),
            Self::ServeSaturated => Some(SERVE_RETRY_AFTER_SECONDS),
            Self::Git(GitError::AdmissionRejected) => Some(ADMISSION_RETRY_AFTER_SECONDS),
            Self::Git(GitError::TransientlyOverCap) => Some(PRESSURE_RETRY_AFTER_SECONDS),
            Self::Store(StoreError::Throttled) | Self::Git(GitError::Throttled) => {
                Some(THROTTLED_RETRY_AFTER_SECONDS)
            }
            Self::BadRequest(_)
            | Self::Store(_)
            | Self::Git(_)
            | Self::ProxyTokenRejected
            | Self::Serialization(_) => None,
        }
    }

    fn into_canonical(self) -> CanonicalError {
        match self {
            Self::BadRequest(e) => RepositoryError::invalid_argument()
                .with_field_violation(bad_request_field(&e), e.to_string(), "INVALID")
                .create(),

            // A partial clone lazily fetches from the promisor remote, so any
            // git step can be rejected by the vendor.
            Self::Store(StoreError::AuthRejected) | Self::Git(GitError::AuthRejected) => {
                CanonicalError::unauthenticated()
                    .with_reason("ORIGIN_CREDENTIALS_REJECTED")
                    .create()
            }
            Self::ProxyTokenRejected => CanonicalError::unauthenticated()
                .with_reason("PROXY_TOKEN_REJECTED")
                .create(),
            Self::Store(StoreError::NotFound) | Self::Git(GitError::NotFound) => {
                RepositoryError::not_found("repository not found at origin")
                    .with_resource("repository")
                    .create()
            }
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
            Self::ServeSaturated => {
                RepositoryError::resource_exhausted("every page-serve slot is taken")
                    .with_quota_violation("serve_concurrency", "all serve slots in use")
                    .with_quota_violation_retry_after_seconds(SERVE_RETRY_AFTER_SECONDS)
                    .create()
            }
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

#[must_use]
pub fn handler_timed_out() -> Response {
    CanonicalError::service_unavailable()
        .with_detail("the request outlived the handler budget")
        .create()
        .into_response()
}

fn internal_error(error: &dyn std::fmt::Display) -> CanonicalError {
    tracing::error!(error = %error, "request failed");
    CanonicalError::internal(error.to_string()).create()
}

fn bad_request_field(error: &BadRequest) -> String {
    match error {
        BadRequest::MissingHeader(name)
        | BadRequest::NotANumber(name)
        | BadRequest::MissingParam(name)
        | BadRequest::MalformedInstant(name) => (*name).to_owned(),
        BadRequest::MalformedToken => "page_token".to_owned(),
        BadRequest::MalformedSha(_) => "sha".to_owned(),
        BadRequest::BadRepoUrl(_) => "repo".to_owned(),
        BadRequest::MalformedQuery => "query".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use axum::body::to_bytes;

    #[test]
    fn serve_saturation_answers_429_with_a_retry_hint() {
        use axum::response::IntoResponse;

        let response = ApiError::ServeSaturated.into_response();
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
        let retry_after = match response.headers().get(header::RETRY_AFTER) {
            Some(value) => value.to_str().unwrap_or_default().to_owned(),
            None => panic!("saturation must tell the caller when to come back"),
        };
        assert_eq!(retry_after, SERVE_RETRY_AFTER_SECONDS.to_string());
    }

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
            (StoreError::OriginUnavailable.into(), StatusCode::NOT_FOUND),
            (GitError::AuthRejected.into(), StatusCode::UNAUTHORIZED),
            (GitError::NotFound.into(), StatusCode::NOT_FOUND),
            (GitError::OriginUnavailable.into(), StatusCode::NOT_FOUND),
            (
                GitError::TooLarge { cap_bytes: 1024 }.into(),
                StatusCode::PAYLOAD_TOO_LARGE,
            ),
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
            (
                GitError::AdmissionRejected.into(),
                StatusCode::TOO_MANY_REQUESTS,
            ),
            (ApiError::ServeSaturated, StatusCode::TOO_MANY_REQUESTS),
            (
                ApiError::Serialization("boom".to_owned()),
                StatusCode::INTERNAL_SERVER_ERROR,
            ),
        ];
        for (error, expected) in cases {
            let label = error.to_string();
            let recorded = error.status_code();
            let (status, _, body) = problem(error).await;
            assert_eq!(status, expected, "for {label}");
            assert_eq!(body["status"], expected.as_u16(), "for {label}");
            assert_eq!(recorded, expected.as_u16(), "recorded status for {label}");
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
