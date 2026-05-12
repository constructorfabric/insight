//! Threshold domain model — server-side threshold evaluation for cell coloring.

use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A threshold rule — configured per metric, per field.
///
/// The query engine evaluates every result row against the metric's thresholds
/// and attaches a `_thresholds` map to the response.
#[derive(Debug, Clone, Serialize, Deserialize)]
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

/// Request to create a threshold.
#[derive(Debug, Deserialize)]
pub struct CreateThresholdRequest {
    pub field_name: String,
    /// Comparison operator: `gt`, `ge`, `lt`, `le`, `eq`.
    pub operator: String,
    pub value: f64,
    /// Result level: `good`, `warning`, `critical`.
    pub level: String,
}

/// Request to update a threshold.
#[derive(Debug, Deserialize)]
pub struct UpdateThresholdRequest {
    pub field_name: Option<String>,
    pub operator: Option<String>,
    pub value: Option<f64>,
    pub level: Option<String>,
}

pub const VALID_OPERATORS: &[&str] = &["gt", "ge", "lt", "le", "eq"];
pub const VALID_LEVELS: &[&str] = &["good", "warning", "critical"];

/// Validate operator and level values.
pub fn validate_threshold(operator: &str, level: &str) -> Result<(), &'static str> {
    if !VALID_OPERATORS.contains(&operator) {
        return Err("operator must be one of: gt, ge, lt, le, eq");
    }
    if !VALID_LEVELS.contains(&level) {
        return Err("level must be one of: good, warning, critical");
    }
    Ok(())
}

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
