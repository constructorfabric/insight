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

#[cfg(test)]
mod tests {
    use super::*;

    type R = Result<(), Box<dyn std::error::Error>>;

    #[test]
    fn each_operator_matches_and_rejects_correctly() {
        assert!(threshold_matches(5.0, "gt", 4.0));
        assert!(!threshold_matches(4.0, "gt", 4.0));
        assert!(threshold_matches(4.0, "ge", 4.0));
        assert!(!threshold_matches(3.9, "ge", 4.0));
        assert!(threshold_matches(3.0, "lt", 4.0));
        assert!(!threshold_matches(4.0, "lt", 4.0));
        assert!(threshold_matches(4.0, "le", 4.0));
        assert!(!threshold_matches(4.1, "le", 4.0));
        assert!(threshold_matches(4.0, "eq", 4.0));
        assert!(!threshold_matches(4.1, "eq", 4.0));
    }

    #[test]
    fn eq_tolerates_floating_point_error() {
        // 0.1 + 0.2 != 0.3 in IEEE-754; the epsilon compare must still match.
        assert!(threshold_matches(0.1 + 0.2, "eq", 0.3));
    }

    #[test]
    fn unknown_operator_never_matches() {
        assert!(!threshold_matches(5.0, "between", 4.0));
        assert!(!threshold_matches(5.0, "", 4.0));
        assert!(!threshold_matches(5.0, "GT", 4.0)); // case-sensitive
    }

    #[test]
    fn valid_sets_match_their_messages() {
        assert_eq!(VALID_OPERATORS, &["gt", "ge", "lt", "le", "eq"]);
        assert_eq!(VALID_LEVELS, &["good", "warning", "critical"]);
        for op in VALID_OPERATORS {
            assert!(INVALID_OPERATOR_MSG.contains(op));
        }
        for lvl in VALID_LEVELS {
            assert!(INVALID_LEVEL_MSG.contains(lvl));
        }
    }

    #[test]
    fn create_request_deserializes() -> R {
        let req: CreateThresholdRequest = serde_json::from_str(
            r#"{"field_name":"score","operator":"ge","value":4.0,"level":"good"}"#,
        )?;
        assert_eq!(req.field_name, "score");
        assert_eq!(req.operator, "ge");
        assert!((req.value - 4.0).abs() < f64::EPSILON);
        assert_eq!(req.level, "good");
        Ok(())
    }

    #[test]
    fn update_request_allows_partial_fields() -> R {
        let req: UpdateThresholdRequest = serde_json::from_str(r#"{"value":9.5}"#)?;
        assert_eq!(req.value, Some(9.5));
        assert!(req.field_name.is_none());
        assert!(req.operator.is_none());
        assert!(req.level.is_none());
        Ok(())
    }
}
