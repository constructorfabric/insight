//! Wire shape and read path for `GET /v1/metric-definitions`.
//!
//! Read-only display listing of the unified metric definitions: every
//! definition visible to the request tenant (product rows plus tenant
//! overrides, tenant row winning per `metric_key`), regardless of
//! `is_enabled` / schema state — the listing doubles as a health surface,
//! so availability is reported (`is_enabled`, `schema_status`) rather than
//! filtered. Computation internals (inputs, computation type, transform)
//! stay off the wire: consumers get the meaning of a metric, not its
//! implementation.

use std::collections::{BTreeMap, HashMap};

use sea_orm::{ConnectionTrait, DatabaseConnection, FromQueryResult, Statement, Value};
use serde::Serialize;
use toolkit_canonical_errors::CanonicalError;
use uuid::Uuid;

use crate::domain::metric_definitions::definition::{MetricDirection, MetricFormat};
use crate::domain::metric_definitions::error_code::{MetricSchemaErrorCode, SchemaStatus};
use crate::domain::metric_definitions::repository::fetch_dimensions;

/// Response body for `GET /v1/metric-definitions`. Metrics are sorted by
/// `metric_key` ascending so the payload is byte-stable for caching and
/// diff tooling.
#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct MetricDefinitionListResponse {
    pub metrics: Vec<MetricDefinitionView>,
}

/// One metric definition, display fields only.
#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct MetricDefinitionView {
    pub metric_key: String,
    pub label: String,
    /// Compact label for dense surfaces; absent when the full label is
    /// already compact enough.
    pub short_label: Option<String>,
    pub description: Option<String>,
    pub explanation: Option<String>,
    pub unit: Option<String>,
    pub format: MetricFormat,
    pub direction: MetricDirection,
    pub dimensions: Vec<String>,
    pub is_enabled: bool,
    pub schema_status: SchemaStatus,
    /// Why `schema_status` is `error`; absent otherwise (the DB enforces the
    /// biconditional).
    pub schema_error_code: Option<MetricSchemaErrorCode>,
    /// Newest `metric_date` ever observed across the definition's input
    /// measures; absent when no observation has ever been seen. Freshness
    /// signal, orthogonal to `schema_status`.
    pub last_observed_date: Option<chrono::NaiveDate>,
}

impl toolkit::api::api_dto::ResponseApiDto for MetricDefinitionListResponse {}

#[derive(Debug, FromQueryResult)]
struct ListingRow {
    definition_id: Uuid,
    tenant_id: Option<Uuid>,
    metric_key: String,
    label: String,
    short_label: Option<String>,
    description: Option<String>,
    explanation: Option<String>,
    unit: Option<String>,
    format: String,
    direction: String,
    is_enabled: bool,
    schema_status: String,
    schema_error_code: Option<String>,
    last_observed_date: Option<chrono::NaiveDate>,
}

pub async fn list_definition_views(
    db: &DatabaseConnection,
    tenant_id: Uuid,
) -> Result<MetricDefinitionListResponse, CanonicalError> {
    let rows = fetch_listing_rows(db, tenant_id)
        .await
        .map_err(|error| db_error(&error))?;
    let selected = select_rows(rows);

    let definition_ids = selected
        .iter()
        .map(|row| row.definition_id)
        .collect::<Vec<_>>();
    let dimensions = fetch_dimensions(db, &definition_ids)
        .await
        .map_err(|error| db_error(&error))?;

    let metrics = build_views(selected, dimensions)?;
    Ok(MetricDefinitionListResponse { metrics })
}

/// Collapse the tenant + product rows per `metric_key` to the one that wins:
/// a tenant-scoped row overrides the product default. Input order is
/// irrelevant; output is sorted by `metric_key` (`BTreeMap` key order).
fn select_rows(rows: Vec<ListingRow>) -> Vec<ListingRow> {
    let mut grouped: BTreeMap<String, Vec<ListingRow>> = BTreeMap::new();
    for row in rows {
        grouped.entry(row.metric_key.clone()).or_default().push(row);
    }
    let mut selected = Vec::with_capacity(grouped.len());
    for (_, mut candidates) in grouped {
        // Tenant override (tenant_id = Some) sorts before the product default.
        candidates.sort_by_key(|row| row.tenant_id.is_none());
        selected.push(candidates.remove(0));
    }
    selected
}

/// Map selected rows to wire views, attaching each row's dimensions and
/// decoding its enum columns. Errors on a row whose stored enum value is not
/// canonical (a corrupt-config invariant, not reachable via the write path).
fn build_views(
    selected: Vec<ListingRow>,
    mut dimensions: HashMap<Uuid, Vec<String>>,
) -> Result<Vec<MetricDefinitionView>, CanonicalError> {
    let mut metrics = Vec::with_capacity(selected.len());
    for row in selected {
        let format = MetricFormat::from_db(&row.format)
            .ok_or_else(|| config_error(&row.metric_key, "format", &row.format))?;
        let direction = MetricDirection::from_db(&row.direction)
            .ok_or_else(|| config_error(&row.metric_key, "direction", &row.direction))?;
        let schema_status = SchemaStatus::from_db(&row.schema_status)
            .ok_or_else(|| config_error(&row.metric_key, "schema_status", &row.schema_status))?;
        let schema_error_code = row
            .schema_error_code
            .as_deref()
            .map(|code| {
                MetricSchemaErrorCode::from_db(code)
                    .ok_or_else(|| config_error(&row.metric_key, "schema_error_code", code))
            })
            .transpose()?;
        metrics.push(MetricDefinitionView {
            metric_key: row.metric_key,
            label: row.label,
            short_label: row.short_label,
            description: row.description,
            explanation: row.explanation,
            unit: row.unit,
            format,
            direction,
            dimensions: dimensions.remove(&row.definition_id).unwrap_or_default(),
            is_enabled: row.is_enabled,
            schema_status,
            schema_error_code,
            last_observed_date: row.last_observed_date,
        });
    }
    Ok(metrics)
}

async fn fetch_listing_rows(
    db: &DatabaseConnection,
    tenant_id: Uuid,
) -> Result<Vec<ListingRow>, sea_orm::DbErr> {
    ListingRow::find_by_statement(Statement::from_sql_and_values(
        db.get_database_backend(),
        "SELECT \
            d.id AS definition_id, \
            d.tenant_id AS tenant_id, \
            d.metric_key AS metric_key, \
            d.label AS label, \
            d.short_label AS short_label, \
            d.description AS description, \
            d.explanation AS explanation, \
            d.unit AS unit, \
            d.format AS format, \
            d.direction AS direction, \
            d.is_enabled AS is_enabled, \
            d.schema_status AS schema_status, \
            d.schema_error_code AS schema_error_code, \
            d.last_observed_date AS last_observed_date \
         FROM metric_definitions d \
         WHERE d.tenant_id IS NULL OR d.tenant_id = ? \
         ORDER BY d.metric_key",
        [Value::Bytes(Some(Box::new(tenant_id.as_bytes().to_vec())))],
    ))
    .all(db)
    .await
}

fn db_error(error: &sea_orm::DbErr) -> CanonicalError {
    tracing::error!(error = %error, "metric definition listing query failed");
    CanonicalError::internal("failed to list metric definitions").create()
}

fn config_error(metric_key: &str, field: &str, value: &str) -> CanonicalError {
    tracing::error!(
        metric_key = %metric_key,
        field = %field,
        value = %value,
        "corrupt metric definition row"
    );
    CanonicalError::internal("corrupt metric definition configuration").create()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(metric_key: &str, tenant_id: Option<Uuid>, label: &str) -> ListingRow {
        ListingRow {
            definition_id: Uuid::now_v7(),
            tenant_id,
            metric_key: metric_key.to_owned(),
            label: label.to_owned(),
            short_label: None,
            description: None,
            explanation: None,
            unit: None,
            format: "integer".to_owned(),
            direction: "higher_is_better".to_owned(),
            is_enabled: true,
            schema_status: "unchecked".to_owned(),
            schema_error_code: None,
            last_observed_date: None,
        }
    }

    #[test]
    fn select_rows_prefers_tenant_override_and_sorts_by_key() {
        let tenant = Uuid::now_v7();
        let rows = vec![
            row("git.commits", None, "product"),
            row("git.commits", Some(tenant), "override"),
            row("ai.cost", None, "product-ai"),
        ];
        let selected = select_rows(rows);
        assert_eq!(
            selected
                .iter()
                .map(|r| r.metric_key.as_str())
                .collect::<Vec<_>>(),
            vec!["ai.cost", "git.commits"]
        );
        let Some(commits) = selected.iter().find(|r| r.metric_key == "git.commits") else {
            panic!("git.commits must be selected");
        };
        assert_eq!(commits.label, "override");
    }

    #[test]
    fn build_views_decodes_columns_and_attaches_dimensions() {
        let mut r = row("git.commits", None, "Commits");
        r.schema_status = "error".to_owned();
        r.schema_error_code = Some("table_not_found".to_owned());
        let id = r.definition_id;
        let dims = HashMap::from([(id, vec!["repo".to_owned()])]);

        let Ok(views) = build_views(vec![r], dims) else {
            panic!("canonical rows must map");
        };
        assert_eq!(views.len(), 1);
        let Some(view) = views.first() else {
            panic!("one view");
        };
        assert_eq!(view.format, MetricFormat::Integer);
        assert_eq!(view.direction, MetricDirection::HigherIsBetter);
        assert_eq!(view.schema_status, SchemaStatus::Error);
        assert_eq!(
            view.schema_error_code,
            Some(MetricSchemaErrorCode::TableNotFound)
        );
        assert_eq!(view.dimensions, vec!["repo".to_owned()]);
    }

    #[test]
    fn build_views_rejects_a_noncanonical_enum_value() {
        let mut r = row("git.commits", None, "Commits");
        r.format = "not-a-format".to_owned();
        assert!(build_views(vec![r], HashMap::new()).is_err());
    }
}
