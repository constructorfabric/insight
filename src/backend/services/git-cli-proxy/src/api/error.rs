use axum::Json;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Serialize;

use crate::engine::store::StoreError;

use super::request::BadRequest;

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error(transparent)]
    BadRequest(#[from] BadRequest),
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error("git failed: {0}")]
    Git(#[from] crate::engine::runner::GitError),
}

#[derive(Serialize)]
struct Body {
    error: String,
    message: String,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, code, retry_after) = self.classify();

        // Detail goes to the log; the wire carries a caller-actionable message
        // only. Internal failures name paths, which never leave the process.
        let message = match &self {
            Self::BadRequest(e) => e.to_string(),
            Self::Store(
                store @ (StoreError::AuthRejected
                | StoreError::NotFound
                | StoreError::Busy { .. }
                | StoreError::SnapshotChanged { .. }),
            ) => store.to_string(),
            Self::Store(internal @ (StoreError::Git(_) | StoreError::Io(_))) => {
                tracing::error!(error = %internal, "request failed");
                "internal error".to_owned()
            }
            Self::Git(internal) => {
                tracing::error!(error = %internal, "request failed");
                "internal error".to_owned()
            }
        };

        let body = Json(Body {
            error: code.to_owned(),
            message,
        });

        let mut response = (status, body).into_response();
        if let Some(seconds) = retry_after
            && let Ok(value) = HeaderValue::from_str(&seconds.to_string())
        {
            response.headers_mut().insert(header::RETRY_AFTER, value);
        }
        response
    }
}

impl ApiError {
    /// Wire mapping the connector's declarative error handler depends on:
    /// `429` is retried with `Retry-After`, `409`/`4xx` are not.
    fn classify(&self) -> (StatusCode, &'static str, Option<u64>) {
        match self {
            Self::BadRequest(_) => (StatusCode::BAD_REQUEST, "bad_request", None),
            Self::Store(StoreError::AuthRejected) => {
                (StatusCode::UNAUTHORIZED, "origin_auth_rejected", None)
            }
            Self::Store(StoreError::NotFound) => (StatusCode::NOT_FOUND, "repo_not_found", None),
            Self::Store(StoreError::Busy { retry_after }) => (
                StatusCode::TOO_MANY_REQUESTS,
                "repo_preparing",
                Some(retry_after.as_secs()),
            ),
            Self::Store(StoreError::SnapshotChanged { .. }) => {
                (StatusCode::CONFLICT, "snapshot_changed", None)
            }
            Self::Store(StoreError::Git(_) | StoreError::Io(_)) | Self::Git(_) => {
                (StatusCode::INTERNAL_SERVER_ERROR, "internal", None)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn maps_every_failure_to_its_documented_status() {
        let cases: Vec<(ApiError, StatusCode, &str)> = vec![
            (
                BadRequest::MissingHeader("x-tenant-id").into(),
                StatusCode::BAD_REQUEST,
                "bad_request",
            ),
            (
                StoreError::AuthRejected.into(),
                StatusCode::UNAUTHORIZED,
                "origin_auth_rejected",
            ),
            (
                StoreError::NotFound.into(),
                StatusCode::NOT_FOUND,
                "repo_not_found",
            ),
            (
                StoreError::Busy {
                    retry_after: Duration::from_secs(30),
                }
                .into(),
                StatusCode::TOO_MANY_REQUESTS,
                "repo_preparing",
            ),
            (
                StoreError::SnapshotChanged { current: 5 }.into(),
                StatusCode::CONFLICT,
                "snapshot_changed",
            ),
            (
                StoreError::Git("boom".to_owned()).into(),
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal",
            ),
        ];
        for (error, expected_status, expected_code) in cases {
            let (status, code, _) = error.classify();
            assert_eq!(status, expected_status, "for {error}");
            assert_eq!(code, expected_code, "for {error}");
        }
    }

    #[test]
    fn busy_carries_retry_after_and_others_do_not() {
        let busy = ApiError::from(StoreError::Busy {
            retry_after: Duration::from_secs(42),
        });
        let response = busy.into_response();
        assert_eq!(
            response
                .headers()
                .get(header::RETRY_AFTER)
                .and_then(|v| v.to_str().ok()),
            Some("42")
        );

        let conflict = ApiError::from(StoreError::SnapshotChanged { current: 2 });
        let response = conflict.into_response();
        assert!(
            response.headers().get(header::RETRY_AFTER).is_none(),
            "a changed snapshot is not retryable by waiting"
        );
    }

    #[tokio::test]
    async fn internal_errors_do_not_leak_paths_to_the_wire() {
        let secret_path = "/data/repos/abc123/repo.git";
        let error = ApiError::from(StoreError::Io(std::io::Error::other(format!(
            "{secret_path}: permission denied"
        ))));
        let (status, code, _) = error.classify();
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(code, "internal");

        let response = error.into_response();
        let bytes = match axum::body::to_bytes(response.into_body(), 4096).await {
            Ok(b) => b,
            Err(e) => panic!("body read failed: {e}"),
        };
        let body = String::from_utf8_lossy(&bytes);
        assert!(
            !body.contains(secret_path),
            "internal detail must stay in the log, got: {body}"
        );
        assert!(body.contains("internal error"), "got: {body}");
    }

    #[tokio::test]
    async fn caller_actionable_errors_keep_their_message() {
        let error = ApiError::from(BadRequest::MissingHeader("x-git-token"));
        let response = error.into_response();
        let bytes = match axum::body::to_bytes(response.into_body(), 4096).await {
            Ok(b) => b,
            Err(e) => panic!("body read failed: {e}"),
        };
        let body = String::from_utf8_lossy(&bytes);
        assert!(
            body.contains("x-git-token"),
            "the caller must learn which header is missing, got: {body}"
        );
    }
}
