//! Metric domain model.

use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A metric definition — an admin-configured SQL query against `ClickHouse`.
///
/// The `query_ref` field holds raw `ClickHouse` SQL. The query engine wraps it
/// as a subquery, appending security filters + `OData` filters as parameterized
/// WHERE clauses.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Serialize)]
pub struct MetricSummary {
    pub id: Uuid,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// Request to create a new metric.
#[derive(Debug, Deserialize)]
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
#[derive(Debug, Deserialize)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TableColumn {
    pub id: Uuid,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub insight_tenant_id: Option<Uuid>,
    pub clickhouse_table: String,
    pub field_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field_description: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    type R = Result<(), Box<dyn std::error::Error>>;

    #[test]
    fn update_description_is_triple_state() -> R {
        // absent → leave unchanged
        let absent: UpdateMetricRequest = serde_json::from_str("{}")?;
        assert_eq!(absent.description, None);
        // explicit null → clear to None
        let null: UpdateMetricRequest = serde_json::from_str(r#"{"description":null}"#)?;
        assert_eq!(null.description, Some(None));
        // value → set
        let val: UpdateMetricRequest = serde_json::from_str(r#"{"description":"hi"}"#)?;
        assert_eq!(val.description, Some(Some("hi".to_owned())));
        Ok(())
    }

    #[test]
    fn metric_omits_none_description_and_keeps_query_ref() -> R {
        let ts: NaiveDateTime = "2026-01-01T00:00:00".parse()?;
        let m = Metric {
            id: Uuid::nil(),
            insight_tenant_id: Uuid::nil(),
            name: "m".to_owned(),
            description: None,
            query_ref: "SELECT 1".to_owned(),
            is_enabled: true,
            created_at: ts,
            updated_at: ts,
        };
        let json = serde_json::to_string(&m)?;
        assert!(!json.contains("description"), "None description omitted: {json}");
        assert!(json.contains("\"query_ref\":\"SELECT 1\""));
        Ok(())
    }

    #[test]
    fn metric_summary_never_exposes_query_ref() -> R {
        let s = MetricSummary {
            id: Uuid::nil(),
            name: "m".to_owned(),
            description: Some("d".to_owned()),
        };
        let json = serde_json::to_string(&s)?;
        assert!(!json.contains("query_ref"), "summary must not leak the SQL");
        assert!(json.contains("\"description\":\"d\""));
        Ok(())
    }

    #[test]
    fn create_request_deserializes_with_optional_description() -> R {
        let r: CreateMetricRequest =
            serde_json::from_str(r#"{"name":"m","query_ref":"SELECT 1"}"#)?;
        assert_eq!(r.name, "m");
        assert_eq!(r.query_ref, "SELECT 1");
        assert!(r.description.is_none());
        Ok(())
    }
}
