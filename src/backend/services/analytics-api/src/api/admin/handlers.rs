//! Axum handlers for the 5 admin-threshold endpoints.
//!
//! All return `Response` directly so the service's `Result<View, Response>`
//! pattern threads through without an `IntoResponse` impl on the error
//! side. List + Get serialize the wire payload via `axum::Json`; mutation
//! handlers do the same on success.

use std::sync::Arc;

use axum::extract::rejection::QueryRejection;
use axum::extract::{Extension, FromRequestParts, Path, Query};
use axum::http::StatusCode;
use axum::http::request::Parts;
use axum::response::{IntoResponse, Response};
use uuid::Uuid;

use crate::api::AppState;
use crate::api::canonical_json::CanonicalJson;
use crate::api::error::ThresholdAdminError;
use crate::domain::admin_threshold::dto::{CreateRequest, ListFilters, UpdateRequest};
use toolkit_security::SecurityContext;

/// `GET /v1/admin/metric-thresholds`.
pub async fn list(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    CanonicalQuery(filters): CanonicalQuery<ListFilters>,
) -> Response {
    match state.admin_threshold.list(&ctx, &filters).await {
        Ok(payload) => axum::Json(payload).into_response(),
        Err(resp) => resp,
    }
}

/// `GET /v1/admin/metric-thresholds/{id}`.
pub async fn get_one(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    Path(id): Path<Uuid>,
) -> Response {
    match state.admin_threshold.get_one(&ctx, id).await {
        Ok(payload) => axum::Json(payload).into_response(),
        Err(resp) => resp,
    }
}

/// `POST /v1/admin/metric-thresholds`.
pub async fn create(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    CanonicalJson(req): CanonicalJson<CreateRequest>,
) -> Response {
    match state.admin_threshold.create(&ctx, &req).await {
        Ok(payload) => (StatusCode::CREATED, axum::Json(payload)).into_response(),
        Err(resp) => resp,
    }
}

/// `PUT /v1/admin/metric-thresholds/{id}`.
pub async fn update(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    Path(id): Path<Uuid>,
    CanonicalJson(req): CanonicalJson<UpdateRequest>,
) -> Response {
    match state.admin_threshold.update(&ctx, id, &req).await {
        Ok(payload) => axum::Json(payload).into_response(),
        Err(resp) => resp,
    }
}

/// `DELETE /v1/admin/metric-thresholds/{id}`. Returns 204 on success per
/// DNA `REST/STATUS_CODES.md` line 10 (delete with no body).
pub async fn delete(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    Path(id): Path<Uuid>,
) -> Response {
    match state.admin_threshold.delete(&ctx, id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(resp) => resp,
    }
}

/// `axum::extract::Query<T>` wrapper that converts the default
/// `QueryRejection` (plain-text body) into the canonical RFC 9457
/// envelope. Same pattern as [`CanonicalJson`] for JSON bodies.
///
/// `T` must be `serde::de::DeserializeOwned + Send`.
pub struct CanonicalQuery<T>(pub T);

impl<S, T> FromRequestParts<S> for CanonicalQuery<T>
where
    T: serde::de::DeserializeOwned + Send,
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        match Query::<T>::from_request_parts(parts, state).await {
            Ok(Query(v)) => Ok(Self(v)),
            Err(rej) => Err(query_rejection_to_response(&rej)),
        }
    }
}

fn query_rejection_to_response(rej: &QueryRejection) -> Response {
    // `QueryRejection` carries a `serde_urlencoded` error; surface it
    // as a canonical 400 `invalid_argument` so the admin contract
    // "`?tenant_id=...` rejected" lands as Problem+JSON instead of
    // Axum's plain-text default. The detailed serde-path of the
    // offending field lands in the server-side log; the wire `detail`
    // stays generic so we don't reflect query strings back to the
    // caller.
    tracing::debug!(error = %rej, "canonical_query: rejection");
    ThresholdAdminError::invalid_argument()
        .with_field_violation(
            "query",
            "query parameters did not match the expected schema",
            "INVALID",
        )
        .create()
        .into_response()
}
