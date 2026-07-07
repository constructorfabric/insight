//! Threshold domain model — server-side threshold evaluation for cell coloring.

use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A threshold rule — configured per metric, per field.
///
/// The query engine evaluates every result row against the metric's thresholds
/// and attaches a `_thresholds` map to the response.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct Threshold {
    pub id: Uuid,
    pub insight_tenant_id: Uuid,
    pub metric_id: Uuid,
    pub field_name: String,
    pub operator: String,
    pub value: f64,
    pub level: String,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

/// Response envelope for `GET /v1/metrics/{id}/thresholds`
/// (`{ "items": [Threshold] }`).
///
/// Docs-only wrapper mirroring the inline `serde_json::json!` shape the list
/// handler emits.
#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct ThresholdListResponse {
    pub items: Vec<Threshold>,
}

/// Request to create a threshold.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct CreateThresholdRequest {
    pub field_name: String,
    /// Comparison operator: `gt`, `ge`, `lt`, `le`, `eq`.
    pub operator: String,
    pub value: f64,
    /// Result level: `good`, `warning`, `critical`.
    pub level: String,
}

/// Request to update a threshold.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct UpdateThresholdRequest {
    pub field_name: Option<String>,
    pub operator: Option<String>,
    pub value: Option<f64>,
    pub level: Option<String>,
}

// Marker traits — `Threshold` / `ThresholdListResponse` are response-side;
// the two `*Request` shapes are request bodies.
impl toolkit::api::api_dto::ResponseApiDto for Threshold {}
impl toolkit::api::api_dto::ResponseApiDto for ThresholdListResponse {}
impl toolkit::api::api_dto::RequestApiDto for CreateThresholdRequest {}
impl toolkit::api::api_dto::RequestApiDto for UpdateThresholdRequest {}

pub const VALID_OPERATORS: &[&str] = &["gt", "ge", "lt", "le", "eq"];
pub const VALID_LEVELS: &[&str] = &["good", "warning", "critical"];

pub const INVALID_OPERATOR_MSG: &str = "operator must be one of: gt, ge, lt, le, eq";
pub const INVALID_LEVEL_MSG: &str = "level must be one of: good, warning, critical";

/// Evaluate a numeric value against a threshold condition.
#[allow(dead_code)] // will be called by query engine when threshold evaluation is wired
pub fn threshold_matches(value: f64, operator: &str, threshold: f64) -> bool {
    match operator {
        "gt" => value > threshold,
        "ge" => value >= threshold,
        "lt" => value < threshold,
        "le" => value <= threshold,
        "eq" => (value - threshold).abs() < f64::EPSILON,
        _ => false,
    }
}
