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

use crate::domain::metric_definitions::builtin::{builtin_metrics, builtin_sources};
use crate::domain::metric_definitions::definition::{MetricDirection, MetricFormat, MetricOrigin};
use crate::domain::metric_definitions::error_code::{MetricSchemaErrorCode, SchemaStatus};
use crate::domain::metric_definitions::repository::{fetch_dimensions, fetch_tags};
use crate::domain::metric_drilldown::{MetricDrilldownCapability, load_capabilities};

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
    /// The single topic this metric belongs to within its family, so a surface
    /// listing a family can partition it into topics rather than only sorting
    /// by name. Exactly one per metric; absent only for metrics that declare
    /// none.
    pub subject: Option<String>,
    pub description: Option<String>,
    pub explanation: Option<String>,
    pub unit: Option<String>,
    pub format: MetricFormat,
    pub direction: MetricDirection,
    pub dimensions: Vec<String>,
    /// Cross-cutting labels a surface can filter or search by; many per metric,
    /// unlike the singular `subject`. Empty when the metric declares none.
    pub tags: Vec<String>,
    pub is_enabled: bool,
    /// `builtin` metrics read managed observation relations; `custom` metrics
    /// execute inline SQL at query time. The validator stamps `schema_status`
    /// and `last_observed_date` from materialized relations only, so for
    /// `custom` those fields stay `unchecked` / absent regardless of data —
    /// readers must not interpret them as "never measured" for custom metrics.
    pub origin: MetricOrigin,
    pub schema_status: SchemaStatus,
    /// Why `schema_status` is `error`; absent otherwise (the DB enforces the
    /// biconditional).
    pub schema_error_code: Option<MetricSchemaErrorCode>,
    /// Newest `metric_date` ever observed across the definition's input
    /// measures; absent when no observation has ever been seen. Freshness
    /// signal, orthogonal to `schema_status`. Not maintained for `custom`
    /// metrics (see `origin`).
    pub last_observed_date: Option<chrono::NaiveDate>,
    /// How many days back from `last_observed_date` the suppliers may still
    /// revise. Absent where the source declares none, and for `custom` metrics,
    /// which read no managed source — absence means "settles on arrival", not
    /// "revised forever". Registry knowledge, not tenant state, so it is read
    /// from the seed rather than stored per row.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision_window_days: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub drilldown: Option<MetricDrilldownCapability>,
}

impl toolkit::api::api_dto::ResponseApiDto for MetricDefinitionListResponse {}

#[derive(Debug, FromQueryResult)]
struct ListingRow {
    definition_id: Uuid,
    tenant_id: Option<Uuid>,
    metric_key: String,
    label: String,
    short_label: Option<String>,
    subject: Option<String>,
    description: Option<String>,
    explanation: Option<String>,
    unit: Option<String>,
    format: String,
    direction: String,
    is_enabled: bool,
    origin: String,
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
    let metric_keys = selected
        .iter()
        .map(|row| row.metric_key.clone())
        .collect::<Vec<_>>();
    let mut capabilities = match load_capabilities(db, tenant_id, &metric_keys).await {
        Ok(capabilities) => capabilities,
        Err(error) => {
            tracing::warn!(error = ?error, "metric drilldown capability load failed");
            HashMap::new()
        }
    };

    let definition_ids = selected
        .iter()
        .map(|row| row.definition_id)
        .collect::<Vec<_>>();
    let dimensions = fetch_dimensions(db, &definition_ids)
        .await
        .map_err(|error| db_error(&error))?;
    let tags = fetch_tags(db, &definition_ids)
        .await
        .map_err(|error| db_error(&error))?;

    let mut metrics = build_views(selected, dimensions, tags)?;
    for metric in &mut metrics {
        metric.drilldown = capabilities.remove(&metric.metric_key);
    }
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
    mut tags: HashMap<Uuid, Vec<String>>,
) -> Result<Vec<MetricDefinitionView>, CanonicalError> {
    let mut metrics = Vec::with_capacity(selected.len());
    let revision_windows = revision_window_by_metric();
    for row in selected {
        let format = MetricFormat::from_db(&row.format)
            .ok_or_else(|| config_error(&row.metric_key, "format", &row.format))?;
        let direction = MetricDirection::from_db(&row.direction)
            .ok_or_else(|| config_error(&row.metric_key, "direction", &row.direction))?;
        let origin = MetricOrigin::from_db(&row.origin)
            .ok_or_else(|| config_error(&row.metric_key, "origin", &row.origin))?;
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
        let revision_window_days = revision_windows.get(row.metric_key.as_str()).copied();
        metrics.push(MetricDefinitionView {
            metric_key: row.metric_key,
            label: row.label,
            short_label: row.short_label,
            subject: row.subject,
            description: row.description,
            explanation: row.explanation,
            unit: row.unit,
            format,
            direction,
            dimensions: dimensions.remove(&row.definition_id).unwrap_or_default(),
            tags: tags.remove(&row.definition_id).unwrap_or_default(),
            is_enabled: row.is_enabled,
            origin,
            schema_status,
            schema_error_code,
            last_observed_date: row.last_observed_date,
            revision_window_days,
            drilldown: None,
        });
    }
    Ok(metrics)
}

/// Each builtin metric's revision window, taken from the source it reads.
///
/// The window belongs to the supplier, not to the tenant, so it comes from the
/// seed rather than from `metric_definitions` — a stored copy would be a second
/// truth to keep in step with the registry. A custom metric reads no managed
/// source and so appears here for no key.
fn revision_window_by_metric() -> HashMap<&'static str, u16> {
    let by_source: HashMap<&str, u16> = builtin_sources()
        .iter()
        .filter_map(|source| {
            source
                .source
                .revision_window_days
                .map(|days| (source.source.key.as_str(), days))
        })
        .collect();
    builtin_metrics()
        .iter()
        .filter_map(|metric| {
            by_source
                .get(metric.source_key.as_str())
                .map(|days| (metric.metric_key.as_str(), *days))
        })
        .collect()
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
            d.subject AS subject, \
            d.description AS description, \
            d.explanation AS explanation, \
            d.unit AS unit, \
            d.format AS format, \
            d.direction AS direction, \
            d.is_enabled AS is_enabled, \
            d.origin AS origin, \
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
            subject: None,
            description: None,
            explanation: None,
            unit: None,
            format: "integer".to_owned(),
            direction: "higher_is_better".to_owned(),
            is_enabled: true,
            origin: "builtin".to_owned(),
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
        r.subject = Some("commits".to_owned());
        r.schema_status = "error".to_owned();
        r.schema_error_code = Some("table_not_found".to_owned());
        let id = r.definition_id;
        let dims = HashMap::from([(id, vec!["repo".to_owned()])]);
        let tags = HashMap::from([(id, vec!["rate".to_owned()])]);

        let Ok(views) = build_views(vec![r], dims, tags) else {
            panic!("canonical rows must map");
        };
        assert_eq!(views.len(), 1);
        let Some(view) = views.first() else {
            panic!("one view");
        };
        assert_eq!(view.format, MetricFormat::Integer);
        assert_eq!(view.direction, MetricDirection::HigherIsBetter);
        assert_eq!(view.origin, MetricOrigin::Builtin);
        assert_eq!(view.schema_status, SchemaStatus::Error);
        assert_eq!(
            view.schema_error_code,
            Some(MetricSchemaErrorCode::TableNotFound)
        );
        assert_eq!(view.dimensions, vec!["repo".to_owned()]);
        assert_eq!(view.subject.as_deref(), Some("commits"));
        assert_eq!(view.tags, vec!["rate".to_owned()]);
    }

    #[test]
    fn build_views_decodes_custom_origin() {
        let mut r = row("team.velocity", None, "Velocity");
        r.origin = "custom".to_owned();

        let Ok(views) = build_views(vec![r], HashMap::new(), HashMap::new()) else {
            panic!("canonical rows must map");
        };
        let Some(view) = views.first() else {
            panic!("one view");
        };
        assert_eq!(view.origin, MetricOrigin::Custom);
        assert_eq!(view.schema_status, SchemaStatus::Unchecked);
        assert_eq!(view.schema_error_code, None);
        assert_eq!(view.last_observed_date, None);
        assert_eq!(view.subject, None);
        assert!(view.tags.is_empty());
    }

    #[test]
    fn build_views_rejects_a_noncanonical_enum_value() {
        let mut r = row("git.commits", None, "Commits");
        r.format = "not-a-format".to_owned();
        assert!(build_views(vec![r], HashMap::new(), HashMap::new()).is_err());

        let mut r = row("git.commits", None, "Commits");
        r.origin = "not-an-origin".to_owned();
        assert!(build_views(vec![r], HashMap::new(), HashMap::new()).is_err());
    }
}
