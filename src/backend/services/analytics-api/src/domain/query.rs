//! Query request/response models — `OData`-style per DNA REST conventions.

use serde::{Deserialize, Serialize};

/// Query request body for `POST /v1/metrics/{id}/query`.
///
/// Uses `OData`-style parameters: `$filter`, `$orderby`, `$select`, `$top`, `$skip`.
#[derive(Debug, Deserialize)]
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
#[derive(Debug, Serialize)]
pub struct QueryResponse {
    pub items: Vec<serde_json::Value>,
    pub page_info: PageInfo,
}

/// Pagination info.
#[derive(Debug, Serialize)]
pub struct PageInfo {
    pub has_next: bool,
    pub cursor: Option<String>,
}
