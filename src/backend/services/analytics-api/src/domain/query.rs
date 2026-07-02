//! Query request/response models — `OData`-style per DNA REST conventions.

use serde::{Deserialize, Serialize};
use toolkit_canonical_errors::Problem;
use uuid::Uuid;

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

#[derive(Debug, Deserialize)]
pub struct BatchQueryItem {
    pub id: Option<String>,
    pub metric_id: Uuid,
    #[serde(flatten)]
    pub query: QueryRequest,
}

#[derive(Debug, Deserialize)]
pub struct BatchQueryRequest {
    pub queries: Vec<BatchQueryItem>,
}

#[derive(Debug, Serialize)]
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

#[derive(Debug, Serialize)]
pub struct BatchQueryResponse {
    pub results: Vec<BatchQueryResult>,
}

#[cfg(test)]
mod tests {
    use super::*;

    type R = Result<(), Box<dyn std::error::Error>>;

    // (Removed `default_top_is_25` — asserting a function returns its own literal
    // tests nothing. The default is still covered, meaningfully, by
    // `query_request_applies_defaults_when_empty` below, which proves the field is
    // actually wired to default_top() through deserialization.)

    #[test]
    fn query_request_maps_odata_params() -> R {
        let q: QueryRequest = serde_json::from_str(
            r#"{"$filter":"metric_date ge '2026-03-01'","$orderby":"metric_date desc","$select":"person_id","$top":50}"#,
        )?;
        assert_eq!(q.filter.as_deref(), Some("metric_date ge '2026-03-01'"));
        assert_eq!(q.orderby.as_deref(), Some("metric_date desc"));
        assert_eq!(q.select.as_deref(), Some("person_id"));
        assert_eq!(q.top, 50);
        assert!(q.skip.is_none());
        Ok(())
    }

    #[test]
    fn query_request_applies_defaults_when_empty() -> R {
        let q: QueryRequest = serde_json::from_str("{}")?;
        assert_eq!(q.top, 25, "$top defaults to default_top()");
        assert!(q.filter.is_none());
        assert!(q.orderby.is_none());
        assert!(q.select.is_none());
        assert!(q.skip.is_none());
        Ok(())
    }

    #[test]
    fn batch_request_flattens_query_into_each_item() -> R {
        let b: BatchQueryRequest = serde_json::from_str(
            r#"{"queries":[{"id":"a","metric_id":"11111111-1111-1111-1111-111111111111","$top":10,"$filter":"x eq 1"}]}"#,
        )?;
        assert_eq!(b.queries.len(), 1);
        let item = &b.queries[0];
        assert_eq!(item.id.as_deref(), Some("a"));
        assert_eq!(item.query.top, 10);
        assert_eq!(item.query.filter.as_deref(), Some("x eq 1"));
        Ok(())
    }

    #[test]
    fn batch_result_ok_serializes_with_lowercase_status_tag() -> R {
        let r = BatchQueryResult::Ok {
            id: Some("a".to_owned()),
            metric_id: Uuid::nil(),
            response: QueryResponse {
                items: vec![],
                page_info: PageInfo {
                    has_next: false,
                    cursor: None,
                },
            },
        };
        let json = serde_json::to_string(&r)?;
        assert!(
            json.contains("\"status\":\"ok\""),
            "tag = lowercase variant: {json}"
        );
        Ok(())
    }
}
