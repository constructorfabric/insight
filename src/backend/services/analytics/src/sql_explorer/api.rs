use axum::extract::{DefaultBodyLimit, Request, State, rejection::JsonRejection};
use axum::http::header::{AUTHORIZATION, CACHE_CONTROL, CONTENT_TYPE, WWW_AUTHENTICATE};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use secrecy::ExposeSecret as _;
use serde::Deserialize;
use sha2::{Digest as _, Sha256};
use subtle::ConstantTimeEq as _;
use toolkit_canonical_errors::{CanonicalError, Http, resource_error};

use super::executor::{MAX_REQUEST_BODY_BYTES, QueryExecutor, QueryFailure};
use crate::config::SqlApiConfig;

#[resource_error("gts.cf.insight.analytics_api.sql_query.v1~")]
struct SqlQueryError;

#[cfg(test)]
mod tests;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct QueryRequest {
    sql: String,
}

pub(super) fn router(config: &SqlApiConfig, executor: QueryExecutor) -> Router {
    let token_hash: [u8; 32] = Sha256::digest(config.token.expose_secret().as_bytes()).into();
    Router::new()
        .route("/api/sql/query", post(query))
        .with_state(executor)
        .layer(DefaultBodyLimit::max(MAX_REQUEST_BODY_BYTES))
        .layer(middleware::from_fn_with_state(token_hash, authenticate))
}

async fn authenticate(
    State(expected): State<[u8; 32]>,
    headers: HeaderMap,
    request: Request,
    next: Next,
) -> Response {
    let mut response = if valid_token(&headers, &expected) {
        next.run(request).await
    } else {
        let mut response = CanonicalError::unauthenticated()
            .with_reason("INVALID_INSTANCE_TOKEN")
            .create()
            .into_response();
        response
            .headers_mut()
            .insert(WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
        response
    };
    response
        .headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

fn valid_token(headers: &HeaderMap, expected: &[u8; 32]) -> bool {
    if headers.get_all(AUTHORIZATION).iter().count() != 1 {
        return false;
    }
    let Some(value) = headers.get(AUTHORIZATION).and_then(|v| v.to_str().ok()) else {
        return false;
    };
    let Some((scheme, token)) = value.split_once(' ') else {
        return false;
    };
    if !scheme.eq_ignore_ascii_case("bearer")
        || token.is_empty()
        || token.len() > 1024
        || !token.bytes().all(|b| b.is_ascii_graphic())
    {
        return false;
    }
    let actual: [u8; 32] = Sha256::digest(token.as_bytes()).into();
    bool::from(actual.ct_eq(expected))
}

async fn query(
    State(executor): State<QueryExecutor>,
    body: Result<Json<QueryRequest>, JsonRejection>,
) -> Result<Response, CanonicalError> {
    let Json(request) = body.map_err(|error| {
        if error.status() == StatusCode::PAYLOAD_TOO_LARGE {
            oversized_input("body", "Request body exceeds limit")
        } else {
            invalid_request("Expected a JSON object containing only a sql string")
        }
    })?;
    let bytes = executor
        .query(request.sql, |result| serde_json::to_vec(&result))
        .await
        .map_err(|error| query_error(&error))?;
    Ok(([(CONTENT_TYPE, "application/json")], bytes).into_response())
}

fn invalid_request(reason: &str) -> CanonicalError {
    SqlQueryError::invalid_argument()
        .with_field_violation("sql", reason, "INVALID")
        .create()
}

fn oversized_input(field: &str, reason: &str) -> CanonicalError {
    SqlQueryError::invalid_argument()
        .with_field_violation(field, reason, "TOO_LARGE")
        .with_override(Http::status_code(413))
        .create()
}

fn query_error(error: &QueryFailure) -> CanonicalError {
    match error {
        QueryFailure::InvalidSql(_) => invalid_request(&error.public_message()),
        QueryFailure::SqlTooLarge => oversized_input("sql", &error.public_message()),
        QueryFailure::Busy | QueryFailure::ResultTooLarge | QueryFailure::ResourceLimit => {
            SqlQueryError::resource_exhausted("Query exceeded resource limits")
                .with_quota_violation("sql_explorer", error.public_message())
                .create()
        }
        QueryFailure::Timeout => SqlQueryError::deadline_exceeded("Query timed out").create(),
        QueryFailure::ClickHouse(_)
        | QueryFailure::InvalidResponse(_)
        | QueryFailure::ProcessingTask(_) => {
            CanonicalError::internal("query execution failed").create()
        }
    }
}
