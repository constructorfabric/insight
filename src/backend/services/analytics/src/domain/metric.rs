//! Metric domain model.

use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A metric definition — an admin-configured SQL query against `ClickHouse`.
///
/// The `query_ref` field holds raw `ClickHouse` SQL. The query engine wraps it
/// as a subquery, appending security filters + `OData` filters as parameterized
/// WHERE clauses.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct Metric {
    pub id: Uuid,
    pub insight_tenant_id: Uuid,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub query_ref: String,
    pub is_enabled: bool,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

/// Summary returned in list endpoints (no `query_ref`).
#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct MetricSummary {
    pub id: Uuid,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// Response envelope for `GET /v1/metrics` (`{ "items": [MetricSummary] }`).
///
/// Docs-only wrapper: the handler emits the same object shape via an inline
/// `serde_json::json!` literal. Existing on the wire; this type just gives the
/// list endpoint a real OpenAPI schema instead of a generic object.
#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct MetricListResponse {
    pub items: Vec<MetricSummary>,
}

/// Request to create a new metric.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct CreateMetricRequest {
    pub name: String,
    pub description: Option<String>,
    pub query_ref: String,
}

/// Request to update a metric.
///
/// `description` uses double-Option to distinguish:
/// - absent field → leave unchanged
/// - explicit `null` → clear to None
/// - `"some text"` → set to Some("some text")
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct UpdateMetricRequest {
    pub name: Option<String>,
    #[allow(clippy::option_option)] // intentional: absent vs null vs value for PATCH semantics
    #[serde(default, deserialize_with = "deserialize_optional_nullable")]
    pub description: Option<Option<String>>,
    pub query_ref: Option<String>,
    pub is_enabled: Option<bool>,
}

/// Deserialize a field that can be absent, null, or a value.
/// - absent → `None` (outer)
/// - `null` → `Some(None)`
/// - `"text"` → `Some(Some("text"))`
#[allow(clippy::option_option)] // intentional: triple-state for PATCH semantics
fn deserialize_optional_nullable<'de, D>(
    deserializer: D,
) -> Result<Option<Option<String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(Some(Option::deserialize(deserializer)?))
}

/// A column in the `ClickHouse` schema catalog.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct TableColumn {
    pub id: Uuid,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub insight_tenant_id: Option<Uuid>,
    pub clickhouse_table: String,
    pub field_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field_description: Option<String>,
}

/// Response envelope for `GET /v1/columns` and `GET /v1/columns/{table}`
/// (`{ "items": [TableColumn] }`).
///
/// Docs-only wrapper mirroring the inline `serde_json::json!` shape the
/// handlers emit — gives the column-list endpoints a real OpenAPI schema.
#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct ColumnListResponse {
    pub items: Vec<TableColumn>,
}

// Marker traits — request vs response side per the toolkit's `api_dto`
// contract. `Metric` is a response only (never deserialized from a request
// body); the two `*Request` shapes are request-only.
impl toolkit::api::api_dto::ResponseApiDto for Metric {}
impl toolkit::api::api_dto::ResponseApiDto for MetricSummary {}
impl toolkit::api::api_dto::ResponseApiDto for MetricListResponse {}
impl toolkit::api::api_dto::ResponseApiDto for TableColumn {}
impl toolkit::api::api_dto::ResponseApiDto for ColumnListResponse {}
impl toolkit::api::api_dto::RequestApiDto for CreateMetricRequest {}
impl toolkit::api::api_dto::RequestApiDto for UpdateMetricRequest {}
