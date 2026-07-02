//! Query request/response models — `OData`-style per DNA REST conventions.

use serde::{Deserialize, Serialize};
use toolkit_canonical_errors::Problem;
use uuid::Uuid;

/// Query request body for `POST /v1/metrics/{id}/query`.
///
/// Uses `OData`-style parameters: `$filter`, `$orderby`, `$select`, `$top`, `$skip`.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct QueryRequest {
    /// `OData` filter expression.
    /// e.g. `"metric_date ge '2026-03-01' and metric_date lt '2026-04-01'"`.
    #[serde(rename = "$filter", default)]
    pub filter: Option<String>,

    /// `OData` ordering expression.
    /// e.g. `"metric_date desc"`.
    #[serde(rename = "$orderby", default)]
    pub orderby: Option<String>,

    /// Comma-separated list of columns to return.
    /// e.g. `"person_id, avg_hours, metric_date"`.
    #[serde(rename = "$select", default)]
    pub select: Option<String>,

    /// Maximum number of rows (default 25, max 200).
    #[serde(rename = "$top", default = "default_top")]
    pub top: u64,

    /// Opaque cursor for keyset pagination (from previous `page_info.cursor`).
    #[serde(rename = "$skip", default)]
    #[allow(dead_code)] // will be consumed by query engine for cursor-based pagination
    pub skip: Option<String>,
}

fn default_top() -> u64 {
    25
}

/// Query response with cursor-based pagination.
///
/// `items` rows carry a per-metric dynamic schema (the `SELECT` columns vary by
/// metric), so each row is an untyped JSON object.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct QueryResponse {
    pub items: Vec<serde_json::Value>,
    pub page_info: PageInfo,
}

/// Pagination info.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct PageInfo {
    pub has_next: bool,
    pub cursor: Option<String>,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct BatchQueryItem {
    pub id: Option<String>,
    pub metric_id: Uuid,
    #[serde(flatten)]
    pub query: QueryRequest,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct BatchQueryRequest {
    pub queries: Vec<BatchQueryItem>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
#[serde(tag = "status", rename_all = "lowercase")]
pub enum BatchQueryResult {
    Ok {
        id: Option<String>,
        metric_id: Uuid,
        #[serde(flatten)]
        response: QueryResponse,
    },
    Error {
        id: Option<String>,
        metric_id: Uuid,
        error: Problem,
    },
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct BatchQueryResponse {
    pub results: Vec<BatchQueryResult>,
}

// Marker traits — `QueryRequest` / `BatchQueryRequest` are request bodies;
// the response shapes are response-side. `QueryResponse` / `PageInfo` /
// `BatchQueryResult` / `BatchQueryItem` are nested inside the top-level
// request/response types and only need `ToSchema` (above), but marking the
// two top-level shapes keeps the wiring explicit.
impl toolkit::api::api_dto::RequestApiDto for QueryRequest {}
impl toolkit::api::api_dto::RequestApiDto for BatchQueryRequest {}
impl toolkit::api::api_dto::ResponseApiDto for QueryResponse {}
impl toolkit::api::api_dto::ResponseApiDto for BatchQueryResponse {}
