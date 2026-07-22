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

use std::collections::BTreeMap;

use sea_orm::{ConnectionTrait, DatabaseConnection, FromQueryResult, Statement, Value};
use serde::Serialize;
use toolkit_canonical_errors::CanonicalError;
use uuid::Uuid;

use crate::domain::metric_definitions::definition::{MetricDirection, MetricFormat};
use crate::domain::metric_definitions::error_code::SchemaStatus;
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
}

pub async fn list_definition_views(
    db: &DatabaseConnection,
    tenant_id: Uuid,
) -> Result<MetricDefinitionListResponse, CanonicalError> {
    let rows = fetch_listing_rows(db, tenant_id)
        .await
        .map_err(|error| db_error(&error))?;

    let mut grouped: BTreeMap<String, Vec<ListingRow>> = BTreeMap::new();
    for row in rows {
        grouped.entry(row.metric_key.clone()).or_default().push(row);
    }

    let mut selected = Vec::with_capacity(grouped.len());
    for (_, mut candidates) in grouped {
        // Tenant override wins over the product-default row for the same key.
        candidates.sort_by_key(|row| row.tenant_id.is_none());
        selected.push(candidates.remove(0));
    }

    let definition_ids = selected
        .iter()
        .map(|row| row.definition_id)
        .collect::<Vec<_>>();
    let mut dimensions = fetch_dimensions(db, &definition_ids)
        .await
        .map_err(|error| db_error(&error))?;

    let mut metrics = Vec::with_capacity(selected.len());
    for row in selected {
        let format = MetricFormat::from_db(&row.format)
            .ok_or_else(|| config_error(&row.metric_key, "format", &row.format))?;
        let direction = MetricDirection::from_db(&row.direction)
            .ok_or_else(|| config_error(&row.metric_key, "direction", &row.direction))?;
        let schema_status = SchemaStatus::from_db(&row.schema_status)
            .ok_or_else(|| config_error(&row.metric_key, "schema_status", &row.schema_status))?;
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
        });
    }

    Ok(MetricDefinitionListResponse { metrics })
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
            d.schema_status AS schema_status \
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
