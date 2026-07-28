use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::Write;

use base64::Engine;
use chrono::NaiveDate;
use sea_orm::{ConnectionTrait, DatabaseConnection, FromQueryResult, Statement, Value};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use toolkit_canonical_errors::CanonicalError;
use uuid::Uuid;

use crate::api::error::MetricError;
use crate::domain::metric_definitions::definition::MetricInputRole;
use crate::domain::metric_definitions::{
    ComputationSpec, EvidenceGranularity, EvidenceRelation, MetricDefinition,
    load_definitions_with_ids,
};
use crate::domain::metric_results::{
    normalize_entity_id, normalize_entity_type, normalize_metric_key,
};

const DEFAULT_PAGE_LIMIT: usize = 100;
const MAX_PAGE_LIMIT: usize = 250;
const MAX_PERIOD_DAYS: i64 = 400;
const MAX_FILTERS: usize = 10;
const MAX_DISPLAY_DIMENSIONS: usize = 10;
const MAX_FILTER_VALUES: usize = 100;
const MAX_FILTER_VALUE_BYTES: usize = 512;
pub const MAX_EXPORT_ROWS: usize = 50_000;
pub const EVIDENCE_QUERY_TIMEOUT_SECS: u64 = 45;
pub const EVIDENCE_QUERY_MEMORY_BYTES: usize = 256 * 1024 * 1024;
pub const EVIDENCE_QUERY_READ_BYTES: usize = 512 * 1024 * 1024;
pub const EVIDENCE_QUERY_RESULT_BYTES: usize = 32 * 1024 * 1024;

#[derive(Debug, Clone, Deserialize, Serialize, utoipa::ToSchema)]
pub struct MetricDrilldownEntity {
    pub r#type: String,
    pub id: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, utoipa::ToSchema)]
pub struct MetricDrilldownPeriod {
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, utoipa::ToSchema)]
pub struct MetricDrilldownFilter {
    pub dimension: String,
    pub values: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, utoipa::ToSchema)]
pub struct MetricDrilldownRequest {
    pub metric_key: String,
    pub entity: MetricDrilldownEntity,
    pub period: MetricDrilldownPeriod,
    #[serde(default)]
    pub filters: Vec<MetricDrilldownFilter>,
    #[serde(default)]
    pub display_dimensions: Vec<String>,
    pub limit: Option<usize>,
    pub cursor: Option<String>,
}

#[derive(Debug, Clone, Copy, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum MetricDrilldownExportFormat {
    Csv,
    Xlsx,
}

#[derive(Debug, Clone, Deserialize, utoipa::ToSchema)]
pub struct MetricDrilldownExportRequest {
    pub metric_key: String,
    pub entity: MetricDrilldownEntity,
    pub period: MetricDrilldownPeriod,
    #[serde(default)]
    pub filters: Vec<MetricDrilldownFilter>,
    #[serde(default)]
    pub display_dimensions: Vec<String>,
    pub format: MetricDrilldownExportFormat,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct MetricDrilldownSelection {
    pub metric_key: String,
    pub entity: MetricDrilldownEntity,
    pub period: MetricDrilldownPeriod,
    pub filters: Vec<MetricDrilldownFilter>,
    pub display_dimensions: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum MetricDrilldownColumnType {
    String,
    Date,
    Number,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct MetricDrilldownColumn {
    pub key: String,
    pub label: String,
    pub r#type: MetricDrilldownColumnType,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct MetricDrilldownRow {
    pub values: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct MetricDrilldownCapability {
    pub granularity: Vec<EvidenceGranularity>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct MetricDrilldownResponse {
    pub selection: MetricDrilldownSelection,
    pub columns: Vec<MetricDrilldownColumn>,
    pub rows: Vec<MetricDrilldownRow>,
    pub next_cursor: Option<String>,
}

impl toolkit::api::api_dto::RequestApiDto for MetricDrilldownRequest {}
impl toolkit::api::api_dto::RequestApiDto for MetricDrilldownExportRequest {}
impl toolkit::api::api_dto::ResponseApiDto for MetricDrilldownResponse {}

#[derive(Debug)]
pub struct ValidatedMetricDrilldown {
    pub selection: MetricDrilldownSelection,
    pub from: NaiveDate,
    pub to: NaiveDate,
    pub limit: usize,
    pub cursor: Option<CursorKey>,
    pub plan: EvidencePlan,
    pub snapshot_id: String,
    pub fingerprint: String,
}

#[derive(Debug, Clone)]
pub struct EvidencePlan {
    pub definition: MetricDefinition,
    pub relation: EvidenceRelation,
    pub source_key: String,
    pub inputs: Vec<EvidenceInput>,
}

#[derive(Debug, Clone)]
pub struct EvidenceInput {
    pub role: MetricInputRole,
    pub measure_key: String,
    pub presentation: EvidencePresentation,
}

#[derive(Debug, Clone)]
pub struct EvidencePresentation {
    pub detail_keys: &'static [&'static str],
    pub show_value: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct EvidenceQueryRow {
    pub role: String,
    pub metric_date: String,
    pub observed_at: String,
    pub source_key: String,
    pub measure_key: String,
    pub record_id: String,
    pub record_kind: String,
    pub contribution: Option<f64>,
    pub numerator: Option<f64>,
    pub denominator: Option<f64>,
    pub subject_key: String,
    pub dimensions_json: String,
    pub details: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct EvidenceDimension {
    key: String,
    value: String,
    label: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CursorKey {
    role: String,
    metric_date: String,
    observed_at: String,
    source_key: String,
    measure_key: String,
    record_id: String,
    record_kind: String,
    subject_key: String,
}

#[derive(Debug, Deserialize, Serialize)]
struct CursorEnvelope {
    version: u8,
    fingerprint: String,
    snapshot_id: String,
    key: CursorKey,
}

struct CommonRequest {
    metric_key: String,
    entity: MetricDrilldownEntity,
    period: MetricDrilldownPeriod,
    filters: Vec<MetricDrilldownFilter>,
    display_dimensions: Vec<String>,
    limit: usize,
    max_limit: usize,
    cursor: Option<String>,
}

#[derive(Debug, FromQueryResult)]
struct EvidenceInputRow {
    input_role: String,
    measure_key: String,
    evidence_granularity: Option<String>,
    source_key: String,
    evidence_ref: Option<String>,
    evidence_schema_status: String,
}

#[derive(Debug, FromQueryResult)]
struct CapabilityRow {
    metric_key: String,
    input_role: String,
    evidence_granularity: Option<String>,
    source_key: String,
    evidence_ref: Option<String>,
    evidence_schema_status: String,
}

#[derive(Debug, Deserialize, clickhouse::Row)]
struct EvidenceSnapshotRow {
    snapshot_id: String,
}

pub async fn load_capabilities(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    metric_keys: &[String],
) -> Result<HashMap<String, MetricDrilldownCapability>, CanonicalError> {
    if metric_keys.is_empty() {
        return Ok(HashMap::new());
    }
    let placeholders = vec!["?"; metric_keys.len()].join(", ");
    let sql = format!(
        "SELECT d.metric_key, i.input_role, m.evidence_granularity, s.source_key, \
                s.evidence_ref, s.evidence_schema_status \
         FROM metric_definitions d \
         INNER JOIN metric_definition_inputs i ON i.metric_definition_id = d.id \
         INNER JOIN metric_source_measures m ON m.id = i.source_measure_id \
         INNER JOIN metric_sources s ON s.id = m.source_id \
         WHERE d.metric_key IN ({placeholders}) \
           AND d.id = COALESCE( \
               (SELECT td.id FROM metric_definitions td WHERE td.metric_key = d.metric_key AND td.tenant_id = ? LIMIT 1), \
               (SELECT pd.id FROM metric_definitions pd WHERE pd.metric_key = d.metric_key AND pd.tenant_id IS NULL LIMIT 1) \
           ) \
           AND d.is_enabled = TRUE AND d.schema_status = 'ok' \
           AND m.is_enabled = TRUE AND s.is_enabled = TRUE \
         ORDER BY d.metric_key, i.input_role, m.measure_key"
    );
    let mut values = metric_keys.iter().map(Value::from).collect::<Vec<_>>();
    values.push(Value::Bytes(Some(Box::new(tenant_id.as_bytes().to_vec()))));
    let rows = CapabilityRow::find_by_statement(Statement::from_sql_and_values(
        db.get_database_backend(),
        sql,
        values,
    ))
    .all(db)
    .await
    .map_err(|error| db_error(&error))?;
    let mut grouped: BTreeMap<String, Vec<CapabilityRow>> = BTreeMap::new();
    for row in rows {
        grouped.entry(row.metric_key.clone()).or_default().push(row);
    }
    let mut capabilities = HashMap::new();
    for (metric_key, rows) in grouped {
        let relation = rows
            .first()
            .and_then(|row| row.evidence_ref.as_deref())
            .and_then(EvidenceRelation::parse);
        let source_key = rows.first().map(|row| row.source_key.as_str());
        let healthy = !rows.is_empty()
            && relation.is_some()
            && source_key.is_some()
            && rows.iter().all(|row| {
                MetricInputRole::from_db(&row.input_role).is_some()
                    && row.evidence_schema_status == "ok"
                    && Some(row.source_key.as_str()) == source_key
                    && row.evidence_ref.as_deref().is_some_and(|value| {
                        relation
                            .as_ref()
                            .is_some_and(|relation| value == relation.source_ref())
                    })
                    && row
                        .evidence_granularity
                        .as_deref()
                        .and_then(EvidenceGranularity::from_db)
                        .is_some()
            });
        if healthy {
            let mut granularity = rows
                .iter()
                .filter_map(|row| {
                    row.evidence_granularity
                        .as_deref()
                        .and_then(EvidenceGranularity::from_db)
                })
                .collect::<Vec<_>>();
            granularity.sort_by_key(|value| value.as_db());
            granularity.dedup();
            capabilities.insert(metric_key, MetricDrilldownCapability { granularity });
        }
    }
    Ok(capabilities)
}

pub async fn validate_request(
    db: &DatabaseConnection,
    ch: &insight_clickhouse::Client,
    tenant_id: Uuid,
    req: MetricDrilldownRequest,
) -> Result<ValidatedMetricDrilldown, CanonicalError> {
    validate_common(
        db,
        ch,
        tenant_id,
        CommonRequest {
            metric_key: req.metric_key,
            entity: req.entity,
            period: req.period,
            filters: req.filters,
            display_dimensions: req.display_dimensions,
            limit: req.limit.unwrap_or(DEFAULT_PAGE_LIMIT),
            max_limit: MAX_PAGE_LIMIT,
            cursor: req.cursor,
        },
    )
    .await
}

pub async fn validate_export_request(
    db: &DatabaseConnection,
    ch: &insight_clickhouse::Client,
    tenant_id: Uuid,
    req: &MetricDrilldownExportRequest,
    limit: usize,
) -> Result<ValidatedMetricDrilldown, CanonicalError> {
    validate_common(
        db,
        ch,
        tenant_id,
        CommonRequest {
            metric_key: req.metric_key.clone(),
            entity: req.entity.clone(),
            period: req.period.clone(),
            filters: req.filters.clone(),
            display_dimensions: req.display_dimensions.clone(),
            limit,
            max_limit: MAX_EXPORT_ROWS + 1,
            cursor: None,
        },
    )
    .await
}

async fn validate_common(
    db: &DatabaseConnection,
    ch: &insight_clickhouse::Client,
    tenant_id: Uuid,
    request: CommonRequest,
) -> Result<ValidatedMetricDrilldown, CanonicalError> {
    let CommonRequest {
        metric_key,
        entity,
        period,
        filters,
        display_dimensions,
        limit,
        max_limit,
        cursor,
    } = request;
    let metric_key = normalize_metric_key("metric_key", &metric_key)?;
    let entity_type = normalize_entity_type(&entity.r#type)?;
    if entity_type != "person" {
        return invalid("entity.type", "only person entities are supported");
    }
    let entity_id = normalize_entity_id(&entity_type, &entity.id);
    if entity_id.is_empty() {
        return invalid("entity.id", "person entity id must not be empty");
    }
    if limit == 0 || limit > max_limit {
        return invalid("limit", format!("limit must be between 1 and {max_limit}"));
    }
    let from = parse_date("period.from", &period.from)?;
    let to = parse_date("period.to", &period.to)?;
    if from > to || (to - from).num_days() >= MAX_PERIOD_DAYS {
        return invalid(
            "period",
            format!("period must be ordered and shorter than {MAX_PERIOD_DAYS} days"),
        );
    }
    let definitions =
        load_definitions_with_ids(db, tenant_id, std::slice::from_ref(&metric_key)).await?;
    let (definition_id, definition) = definitions.get(&metric_key).cloned().ok_or_else(|| {
        MetricError::not_found("metric definition not found")
            .with_resource(&metric_key)
            .create()
    })?;
    if definition.base.entity_type != entity_type {
        return invalid(
            "entity.type",
            "entity type does not match metric definition",
        );
    }
    let filters = normalize_filters(&definition, filters)?;
    let display_dimensions = normalize_display_dimensions(&definition, display_dimensions)?;
    let plan = load_evidence_plan(db, definition_id, definition).await?;
    let snapshot_id = evidence_snapshot_id(ch, &plan.relation).await?;
    let selection = MetricDrilldownSelection {
        metric_key,
        entity: MetricDrilldownEntity {
            r#type: entity_type,
            id: entity_id,
        },
        period: MetricDrilldownPeriod {
            from: from.to_string(),
            to: to.to_string(),
        },
        filters,
        display_dimensions,
    };
    let fingerprint = selection_fingerprint(tenant_id, &selection)?;
    let cursor = match cursor {
        Some(value) => {
            let envelope = decode_cursor(&value)?;
            verify_evidence_snapshot(ch, &plan.relation, &envelope.snapshot_id).await?;
            if envelope.fingerprint != fingerprint {
                return invalid("cursor", "cursor does not match the metric selection");
            }
            Some(envelope.key)
        }
        None => None,
    };
    Ok(ValidatedMetricDrilldown {
        selection,
        from,
        to,
        limit,
        cursor,
        plan,
        snapshot_id,
        fingerprint,
    })
}

async fn load_evidence_plan(
    db: &DatabaseConnection,
    definition_id: Uuid,
    definition: MetricDefinition,
) -> Result<EvidencePlan, CanonicalError> {
    let rows = EvidenceInputRow::find_by_statement(Statement::from_sql_and_values(
        db.get_database_backend(),
        "SELECT i.input_role, m.measure_key, m.evidence_granularity, s.source_key, \
                s.evidence_ref, s.evidence_schema_status \
         FROM metric_definition_inputs i \
         INNER JOIN metric_source_measures m ON m.id = i.source_measure_id \
         INNER JOIN metric_sources s ON s.id = m.source_id \
         WHERE i.metric_definition_id = ? AND m.is_enabled = TRUE AND s.is_enabled = TRUE \
         ORDER BY i.input_role, m.measure_key",
        [Value::Bytes(Some(Box::new(
            definition_id.as_bytes().to_vec(),
        )))],
    ))
    .all(db)
    .await
    .map_err(|error| db_error(&error))?;
    if rows.is_empty() || rows.iter().any(|row| row.evidence_schema_status != "ok") {
        return Err(evidence_unavailable());
    }
    let evidence_ref = rows[0]
        .evidence_ref
        .as_deref()
        .and_then(EvidenceRelation::parse);
    let Some(relation) = evidence_ref else {
        return Err(evidence_unavailable());
    };
    let source_key = rows[0].source_key.clone();
    if rows.iter().any(|row| {
        row.source_key != source_key
            || row.evidence_ref.as_deref() != Some(relation.source_ref())
            || row.evidence_granularity.is_none()
    }) {
        return Err(evidence_unavailable());
    }
    let inputs = rows
        .into_iter()
        .map(|row| {
            let role = MetricInputRole::from_db(&row.input_role).ok_or_else(config_error)?;
            let granularity = row
                .evidence_granularity
                .as_deref()
                .and_then(EvidenceGranularity::from_db)
                .ok_or_else(config_error)?;
            Ok(EvidenceInput {
                role,
                presentation: evidence_presentation(&source_key, &row.measure_key, granularity),
                measure_key: row.measure_key,
            })
        })
        .collect::<Result<Vec<_>, CanonicalError>>()?;
    Ok(EvidencePlan {
        definition,
        relation,
        source_key,
        inputs,
    })
}

pub fn compile_query(
    req: &ValidatedMetricDrilldown,
) -> Result<(String, Vec<String>), CanonicalError> {
    if matches!(req.plan.definition.spec, ComputationSpec::Ratio { .. }) {
        return compile_ratio_query(req);
    }
    Ok(compile_value_query(req))
}

fn compile_value_query(req: &ValidatedMetricDrilldown) -> (String, Vec<String>) {
    let (database, table) = req.plan.relation.table_ref();
    let mut params = Vec::new();
    let measures = req
        .plan
        .inputs
        .iter()
        .map(|_| "?")
        .collect::<Vec<_>>()
        .join(", ");
    let role_expr = role_expression(&req.plan.inputs);
    for input in &req.plan.inputs {
        params.push(input.measure_key.clone());
        params.push(input.role.as_db().to_owned());
    }
    params.extend([
        req.plan.source_key.clone(),
        req.selection.entity.r#type.clone(),
        req.selection.entity.id.clone(),
        req.from.to_string(),
        req.to.to_string(),
    ]);
    params.extend(
        req.plan
            .inputs
            .iter()
            .map(|input| input.measure_key.clone()),
    );
    let mut filter_sql = String::new();
    for filter in &req.selection.filters {
        let placeholders = vec!["?"; filter.values.len()].join(", ");
        let _ = write!(
            filter_sql,
            " AND indexOf(evidence.dimensions.1, ?) > 0 AND evidence.dimensions.2[indexOf(evidence.dimensions.1, ?)] IN ({placeholders})"
        );
        params.push(filter.dimension.clone());
        params.push(filter.dimension.clone());
        params.extend(filter.values.iter().cloned());
    }
    let mut cursor_sql = String::new();
    if let Some(cursor) = &req.cursor {
        cursor_sql.push_str(
            " AND tuple(role, toString(evidence.metric_date), ifNull(toString(evidence.observed_at), ''), evidence.source_key, evidence.measure_key, evidence.record_id, evidence.record_kind, ifNull(evidence.subject_key, '')) > tuple(?, ?, ?, ?, ?, ?, ?, ?)",
        );
        params.extend([
            cursor.role.clone(),
            cursor.metric_date.clone(),
            cursor.observed_at.clone(),
            cursor.source_key.clone(),
            cursor.measure_key.clone(),
            cursor.record_id.clone(),
            cursor.record_kind.clone(),
            cursor.subject_key.clone(),
        ]);
    }
    let limit = req.limit + 1;
    let sql = format!(
        "WITH {role_expr} AS role \
         SELECT role, toString(evidence.metric_date) AS metric_date, ifNull(toString(evidence.observed_at), '') AS observed_at, \
                evidence.source_key, evidence.measure_key, evidence.record_id, evidence.record_kind, \
                evidence.contribution, CAST(NULL AS Nullable(Float64)) AS numerator, \
                CAST(NULL AS Nullable(Float64)) AS denominator, \
                ifNull(evidence.subject_key, '') AS subject_key, \
                toJSONString(evidence.dimensions) AS dimensions_json, evidence.details \
         FROM {database}.{table} AS evidence \
         WHERE evidence.source_key = ? AND evidence.entity_type = ? AND evidence.entity_id = ? \
           AND evidence.metric_date >= toDate(?) AND evidence.metric_date <= toDate(?) \
           AND evidence.measure_key IN ({measures}){filter_sql}{cursor_sql} \
         ORDER BY role, metric_date, ifNull(toString(observed_at), ''), source_key, measure_key, record_id, record_kind, ifNull(subject_key, '') \
         LIMIT {limit}"
    );
    (sql, params)
}

fn compile_ratio_query(
    req: &ValidatedMetricDrilldown,
) -> Result<(String, Vec<String>), CanonicalError> {
    let (database, table) = req.plan.relation.table_ref();
    let numerator = req
        .plan
        .inputs
        .iter()
        .find(|input| input.role == MetricInputRole::Numerator)
        .ok_or_else(config_error)?;
    let denominator = req
        .plan
        .inputs
        .iter()
        .find(|input| input.role == MetricInputRole::Denominator)
        .ok_or_else(config_error)?;
    let mut params = vec![
        numerator.measure_key.clone(),
        denominator.measure_key.clone(),
        req.plan.source_key.clone(),
        req.selection.entity.r#type.clone(),
        req.selection.entity.id.clone(),
        req.from.to_string(),
        req.to.to_string(),
        numerator.measure_key.clone(),
        denominator.measure_key.clone(),
    ];
    let mut filter_sql = String::new();
    for filter in &req.selection.filters {
        let placeholders = vec!["?"; filter.values.len()].join(", ");
        let _ = write!(
            filter_sql,
            " AND indexOf(evidence.dimensions.1, ?) > 0 AND evidence.dimensions.2[indexOf(evidence.dimensions.1, ?)] IN ({placeholders})"
        );
        params.push(filter.dimension.clone());
        params.push(filter.dimension.clone());
        params.extend(filter.values.iter().cloned());
    }
    let mut cursor_sql = String::new();
    if let Some(cursor) = &req.cursor {
        cursor_sql.push_str(
            " WHERE tuple(role, metric_date, observed_at, source_key, measure_key, record_id, record_kind, subject_key) > tuple(?, ?, ?, ?, ?, ?, ?, ?)",
        );
        params.extend([
            cursor.role.clone(),
            cursor.metric_date.clone(),
            cursor.observed_at.clone(),
            cursor.source_key.clone(),
            cursor.measure_key.clone(),
            cursor.record_id.clone(),
            cursor.record_kind.clone(),
            cursor.subject_key.clone(),
        ]);
    }
    let limit = req.limit + 1;
    let sql = format!(
        "SELECT * FROM (\
            SELECT 'value' AS role, toString(evidence.metric_date) AS metric_date, \
                   '' AS observed_at, \
                   any(evidence.source_key) AS source_key, '' AS measure_key, \
                   toString(evidence.metric_date) AS record_id, 'daily_ratio' AS record_kind, \
                   CAST(NULL AS Nullable(Float64)) AS contribution, \
                   sumIf(ifNull(evidence.contribution, 0), evidence.measure_key = ?) AS numerator, \
                   sumIf(ifNull(evidence.contribution, 0), evidence.measure_key = ?) AS denominator, \
                   '' AS subject_key, any(toJSONString(evidence.dimensions)) AS dimensions_json, \
                   CAST(map() AS Map(String, String)) AS details \
            FROM {database}.{table} AS evidence \
            WHERE evidence.source_key = ? AND evidence.entity_type = ? AND evidence.entity_id = ? \
              AND evidence.metric_date >= toDate(?) AND evidence.metric_date <= toDate(?) \
              AND evidence.measure_key IN (?, ?){filter_sql} \
            GROUP BY evidence.metric_date\
         ){cursor_sql} \
         ORDER BY role, metric_date, observed_at, source_key, measure_key, record_id, record_kind, subject_key \
         LIMIT {limit}"
    );
    Ok((sql, params))
}

fn role_expression(inputs: &[EvidenceInput]) -> String {
    let branches = inputs
        .iter()
        .map(|_| "evidence.measure_key = ?, ?")
        .collect::<Vec<_>>()
        .join(", ");
    format!("multiIf({branches}, 'value')")
}

pub fn build_response(
    req: &ValidatedMetricDrilldown,
    mut rows: Vec<EvidenceQueryRow>,
) -> Result<MetricDrilldownResponse, CanonicalError> {
    let next_cursor = if rows.len() > req.limit {
        rows.truncate(req.limit);
        rows.last()
            .map(|row| encode_cursor(&req.fingerprint, &req.snapshot_id, row))
            .transpose()?
    } else {
        None
    };
    let (columns, rows) = presentation(
        &rows,
        &req.plan,
        &req.selection.filters,
        &req.selection.display_dimensions,
    )?;
    Ok(MetricDrilldownResponse {
        selection: req.selection.clone(),
        columns,
        rows,
        next_cursor,
    })
}

pub fn presentation(
    rows: &[EvidenceQueryRow],
    plan: &EvidencePlan,
    filters: &[MetricDrilldownFilter],
    display_dimensions: &[String],
) -> Result<(Vec<MetricDrilldownColumn>, Vec<MetricDrilldownRow>), CanonicalError> {
    let details = rows
        .iter()
        .map(|row| row.details.as_object().ok_or_else(config_error))
        .collect::<Result<Vec<_>, _>>()?;
    let ratio = matches!(plan.definition.spec, ComputationSpec::Ratio { .. });
    let display_dimensions = if ratio { &[] } else { display_dimensions };
    let dimensions = presentation_dimensions(rows)?;
    let mut detail_keys = if ratio {
        BTreeSet::new()
    } else {
        plan.inputs
            .iter()
            .flat_map(|input| input.presentation.detail_keys)
            .map(|key| (*key).to_owned())
            .collect::<BTreeSet<_>>()
    };
    let dimension_keys = filters
        .iter()
        .filter(|filter| filter.values.len() == 1)
        .map(|filter| filter.dimension.clone())
        .chain(display_dimensions.iter().cloned())
        .collect::<BTreeSet<_>>();
    detail_keys.extend(dimension_keys);
    let include_value = !ratio
        && plan
            .inputs
            .iter()
            .any(|input| input.presentation.show_value);
    let mut ordered_keys = Vec::new();
    if detail_keys.remove("ref") {
        ordered_keys.push("ref".to_owned());
    }
    if detail_keys.remove("title") {
        ordered_keys.push("title".to_owned());
    }
    for key in ["repository", "author"] {
        if detail_keys.remove(key) {
            ordered_keys.push(key.to_owned());
        }
    }
    ordered_keys.extend(detail_keys);
    ordered_keys.push("date".to_owned());
    if ratio {
        ordered_keys.push("numerator".to_owned());
        ordered_keys.push("denominator".to_owned());
    } else if include_value {
        ordered_keys.push("value".to_owned());
    }

    let columns = ordered_keys
        .iter()
        .map(|key| presentation_column(key, plan))
        .collect();
    let projected_rows = rows
        .iter()
        .zip(details)
        .zip(dimensions)
        .map(|((row, details), dimensions)| {
            let mut values = BTreeMap::new();
            for key in &ordered_keys {
                let value = match key.as_str() {
                    "date" => row.metric_date.clone().into(),
                    "value" => {
                        serde_json::to_value(row.contribution).map_err(|_| config_error())?
                    }
                    "numerator" => {
                        serde_json::to_value(row.numerator).map_err(|_| config_error())?
                    }
                    "denominator" => {
                        serde_json::to_value(row.denominator).map_err(|_| config_error())?
                    }
                    _ => details
                        .get(key)
                        .filter(|value| visible_value(value))
                        .cloned()
                        .or_else(|| {
                            dimensions
                                .iter()
                                .find(|dimension| dimension.key == *key)
                                .map(|dimension| {
                                    serde_json::Value::from(
                                        dimension
                                            .label
                                            .as_deref()
                                            .filter(|label| !label.trim().is_empty())
                                            .unwrap_or(&dimension.value),
                                    )
                                })
                        })
                        .unwrap_or(serde_json::Value::Null),
                };
                values.insert(key.clone(), normalize_presentation_value(key, value));
            }
            Ok(MetricDrilldownRow { values })
        })
        .collect::<Result<Vec<_>, CanonicalError>>()?;

    Ok((columns, projected_rows))
}

fn presentation_dimensions(
    rows: &[EvidenceQueryRow],
) -> Result<Vec<Vec<EvidenceDimension>>, CanonicalError> {
    rows.iter()
        .map(|row| {
            serde_json::from_str::<Vec<EvidenceDimension>>(&row.dimensions_json)
                .map_err(|_| config_error())
        })
        .collect()
}

fn presentation_column(key: &str, plan: &EvidencePlan) -> MetricDrilldownColumn {
    let (label, r#type) = match key {
        "ref" => ("Ref".to_owned(), MetricDrilldownColumnType::String),
        "title" => ("Title".to_owned(), MetricDrilldownColumnType::String),
        "repository" => ("Repository".to_owned(), MetricDrilldownColumnType::String),
        "author" => ("Author".to_owned(), MetricDrilldownColumnType::String),
        "date" => ("Date".to_owned(), MetricDrilldownColumnType::Date),
        "value" => ("Value".to_owned(), MetricDrilldownColumnType::Number),
        "numerator" => (
            input_label(plan, MetricInputRole::Numerator),
            MetricDrilldownColumnType::Number,
        ),
        "denominator" => (
            input_label(plan, MetricInputRole::Denominator),
            MetricDrilldownColumnType::Number,
        ),
        "lines_added" => ("Lines added".to_owned(), MetricDrilldownColumnType::Number),
        "lines_removed" => (
            "Lines removed".to_owned(),
            MetricDrilldownColumnType::Number,
        ),
        "issue_type" => ("Issue type".to_owned(), MetricDrilldownColumnType::String),
        _ => (humanize_field_name(key), MetricDrilldownColumnType::String),
    };
    MetricDrilldownColumn {
        key: key.to_owned(),
        label,
        r#type,
    }
}

fn evidence_presentation(
    source_key: &str,
    measure_key: &str,
    granularity: EvidenceGranularity,
) -> EvidencePresentation {
    match (source_key, measure_key) {
        ("git", "commit_count" | "commit_change_size") => EvidencePresentation {
            detail_keys: &[
                "ref",
                "title",
                "repository",
                "author",
                "lines_added",
                "lines_removed",
            ],
            show_value: false,
        },
        ("git", "pr_created" | "pr_created_merged" | "pr_merged") => EvidencePresentation {
            detail_keys: &["ref", "title", "repository", "author"],
            show_value: false,
        },
        ("git", "pr_cycle_hours" | "pr_change_size") => EvidencePresentation {
            detail_keys: &["ref", "title", "repository", "author"],
            show_value: true,
        },
        (
            "task",
            "tasks_closed" | "bugs_fixed" | "due_date_on_time" | "due_date_with_due" | "late_count",
        ) => EvidencePresentation {
            detail_keys: &["ref", "issue_type"],
            show_value: false,
        },
        ("task", _) if granularity == EvidenceGranularity::Event => EvidencePresentation {
            detail_keys: &["ref", "issue_type"],
            show_value: true,
        },
        ("wiki", "pages_created") => EvidencePresentation {
            detail_keys: &["ref", "title"],
            show_value: false,
        },
        _ => EvidencePresentation {
            detail_keys: &[],
            show_value: granularity != EvidenceGranularity::Event,
        },
    }
}

fn input_label(plan: &EvidencePlan, role: MetricInputRole) -> String {
    plan.inputs
        .iter()
        .find(|input| input.role == role)
        .map_or_else(
            || humanize_field_name(role.as_db()),
            |input| humanize_field_name(&input.measure_key),
        )
}

fn visible_value(value: &serde_json::Value) -> bool {
    !value.is_null() && value.as_str().is_none_or(|value| !value.trim().is_empty())
}

fn normalize_presentation_value(key: &str, value: serde_json::Value) -> serde_json::Value {
    if matches!(key, "lines_added" | "lines_removed")
        && let Some(value) = value.as_str().and_then(|value| value.parse::<f64>().ok())
    {
        return serde_json::Value::from(value);
    }
    value
}

fn humanize_field_name(key: &str) -> String {
    let label = key.replace('_', " ");
    let mut characters = label.chars();
    match characters.next() {
        Some(first) => first.to_uppercase().chain(characters).collect(),
        None => label,
    }
}

fn normalize_filters(
    definition: &MetricDefinition,
    filters: Vec<MetricDrilldownFilter>,
) -> Result<Vec<MetricDrilldownFilter>, CanonicalError> {
    if filters.len() > MAX_FILTERS {
        return invalid("filters", format!("at most {MAX_FILTERS} filters"));
    }
    let mut normalized = Vec::with_capacity(filters.len());
    for filter in filters {
        let dimension = filter.dimension.trim();
        if definition.allowed_dimension(dimension).is_none() {
            return invalid(
                "filters.dimension",
                format!("dimension {dimension} is not declared by the metric"),
            );
        }
        if filter.values.is_empty() || filter.values.len() > MAX_FILTER_VALUES {
            return invalid(
                "filters.values",
                format!("between 1 and {MAX_FILTER_VALUES} values are required"),
            );
        }
        let mut values = filter
            .values
            .into_iter()
            .map(|value| value.trim().to_owned())
            .collect::<Vec<_>>();
        if values
            .iter()
            .any(|value| value.is_empty() || value.len() > MAX_FILTER_VALUE_BYTES)
        {
            return invalid("filters.values", "filter value is empty or too long");
        }
        values.sort();
        values.dedup();
        normalized.push(MetricDrilldownFilter {
            dimension: dimension.to_owned(),
            values,
        });
    }
    normalized.sort_by(|left, right| left.dimension.cmp(&right.dimension));
    if normalized
        .windows(2)
        .any(|pair| pair[0].dimension == pair[1].dimension)
    {
        return invalid("filters", "duplicate dimension filter");
    }
    Ok(normalized)
}

fn normalize_display_dimensions(
    definition: &MetricDefinition,
    dimensions: Vec<String>,
) -> Result<Vec<String>, CanonicalError> {
    if dimensions.len() > MAX_DISPLAY_DIMENSIONS {
        return invalid(
            "display_dimensions",
            format!("at most {MAX_DISPLAY_DIMENSIONS} display dimensions"),
        );
    }
    let mut normalized = dimensions
        .into_iter()
        .map(|dimension| dimension.trim().to_owned())
        .collect::<Vec<_>>();
    if normalized.iter().any(String::is_empty) {
        return invalid("display_dimensions", "display dimension is empty");
    }
    for dimension in &normalized {
        if definition.allowed_dimension(dimension).is_none() {
            return invalid(
                "display_dimensions",
                format!("dimension {dimension} is not declared by the metric"),
            );
        }
    }
    normalized.sort();
    normalized.dedup();
    Ok(normalized)
}

fn selection_fingerprint(
    tenant_id: Uuid,
    selection: &MetricDrilldownSelection,
) -> Result<String, CanonicalError> {
    let bytes = serde_json::to_vec(&(tenant_id, selection)).map_err(|_| config_error())?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

pub async fn verify_evidence_snapshot(
    ch: &insight_clickhouse::Client,
    relation: &EvidenceRelation,
    expected: &str,
) -> Result<(), CanonicalError> {
    let current = evidence_snapshot_id(ch, relation).await?;
    if current != expected {
        return Err(MetricError::failed_precondition()
            .with_precondition_violation(
                "metric evidence snapshot",
                "Metric evidence was rebuilt while the request was running.",
                "EVIDENCE_SNAPSHOT_EXPIRED",
            )
            .create());
    }
    Ok(())
}

async fn evidence_snapshot_id(
    ch: &insight_clickhouse::Client,
    relation: &EvidenceRelation,
) -> Result<String, CanonicalError> {
    let (database, table) = relation.table_ref();
    ch.query(
        "SELECT toString(uuid) AS snapshot_id \
         FROM system.tables WHERE database = ? AND name = ?",
    )
    .bind(database)
    .bind(table)
    .fetch_one::<EvidenceSnapshotRow>()
    .await
    .map(|row| row.snapshot_id)
    .map_err(|error| {
        tracing::error!(
            error = %error,
            database,
            table,
            "metric evidence snapshot lookup failed"
        );
        evidence_unavailable()
    })
}

fn encode_cursor(
    fingerprint: &str,
    snapshot_id: &str,
    row: &EvidenceQueryRow,
) -> Result<String, CanonicalError> {
    let envelope = CursorEnvelope {
        version: 1,
        fingerprint: fingerprint.to_owned(),
        snapshot_id: snapshot_id.to_owned(),
        key: CursorKey {
            role: row.role.clone(),
            metric_date: row.metric_date.clone(),
            observed_at: row.observed_at.clone(),
            source_key: row.source_key.clone(),
            measure_key: row.measure_key.clone(),
            record_id: row.record_id.clone(),
            record_kind: row.record_kind.clone(),
            subject_key: row.subject_key.clone(),
        },
    };
    let bytes = serde_json::to_vec(&envelope).map_err(|_| config_error())?;
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes))
}

fn decode_cursor(value: &str) -> Result<CursorEnvelope, CanonicalError> {
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(value)
        .map_err(|_| invalid_error("cursor", "cursor is malformed"))?;
    let envelope: CursorEnvelope = serde_json::from_slice(&bytes)
        .map_err(|_| invalid_error("cursor", "cursor is malformed"))?;
    if envelope.version != 1 {
        return invalid("cursor", "cursor version is unsupported");
    }
    Ok(envelope)
}

fn parse_date(field: &str, value: &str) -> Result<NaiveDate, CanonicalError> {
    NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .map_err(|_| invalid_error(field, "date must use YYYY-MM-DD"))
}

fn invalid<T>(field: &str, description: impl Into<String>) -> Result<T, CanonicalError> {
    Err(invalid_error(field, description))
}

fn invalid_error(field: &str, description: impl Into<String>) -> CanonicalError {
    MetricError::invalid_argument()
        .with_field_violation(field, description.into(), "INVALID")
        .create()
}

fn evidence_unavailable() -> CanonicalError {
    MetricError::failed_precondition()
        .with_precondition_violation(
            "metric evidence",
            "Evidence is not available for this metric.",
            "EVIDENCE_UNAVAILABLE",
        )
        .create()
}

fn db_error(error: &sea_orm::DbErr) -> CanonicalError {
    tracing::error!(error = %error, "metric drilldown metadata query failed");
    CanonicalError::internal("failed to load metric evidence metadata").create()
}

fn config_error() -> CanonicalError {
    CanonicalError::internal("corrupt metric evidence configuration").create()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::metric_definitions::definition::{
        MetricBase, MetricDirection, MetricFormat, MetricInput, ObservationRelation,
    };

    fn input(role: MetricInputRole, measure_key: &str) -> MetricInput {
        MetricInput {
            role,
            observation_relation: ObservationRelation::parse("git_metric_observations")
                .unwrap_or_else(|| panic!("observation relation must parse")),
            source_key: "git".to_owned(),
            measure_key: measure_key.to_owned(),
        }
    }

    fn definition(spec: ComputationSpec, dimensions: &[&str]) -> MetricDefinition {
        MetricDefinition {
            base: MetricBase {
                key: "git.example".to_owned(),
                label: "Example".to_owned(),
                short_label: None,
                description: None,
                explanation: None,
                entity_type: "person".to_owned(),
                format: MetricFormat::Integer,
                unit: None,
                direction: MetricDirection::Neutral,
                peer_cohort_key: None,
                allowed_dimensions: dimensions.iter().map(|value| (*value).to_owned()).collect(),
            },
            spec,
            transform: None,
        }
    }

    fn plan(spec: ComputationSpec, inputs: Vec<EvidenceInput>) -> EvidencePlan {
        EvidencePlan {
            definition: definition(spec, &["repository", "category"]),
            relation: EvidenceRelation::parse("git_metric_evidence")
                .unwrap_or_else(|| panic!("evidence relation must parse")),
            source_key: "git".to_owned(),
            inputs,
        }
    }

    fn row() -> EvidenceQueryRow {
        EvidenceQueryRow {
            role: "value".to_owned(),
            metric_date: "2026-07-01".to_owned(),
            observed_at: "2026-07-01 10:00:00".to_owned(),
            source_key: "git".to_owned(),
            measure_key: "commit_count".to_owned(),
            record_id: "abc123".to_owned(),
            record_kind: "commit".to_owned(),
            contribution: Some(1.0),
            numerator: None,
            denominator: None,
            subject_key: String::new(),
            dimensions_json: r#"[{"key":"repository","value":"repo","label":"Repository"},{"key":"category","value":"code","label":null}]"#.to_owned(),
            details: serde_json::json!({
                "ref": "abc123",
                "title": "Change",
                "repository": "org/repo",
                "author": "Developer",
                "lines_added": "12",
                "lines_removed": "3"
            }),
        }
    }

    fn validated(plan: EvidencePlan) -> ValidatedMetricDrilldown {
        let selection = MetricDrilldownSelection {
            metric_key: plan.definition.key().to_owned(),
            entity: MetricDrilldownEntity {
                r#type: "person".to_owned(),
                id: "person@example.com".to_owned(),
            },
            period: MetricDrilldownPeriod {
                from: "2026-07-01".to_owned(),
                to: "2026-07-31".to_owned(),
            },
            filters: vec![MetricDrilldownFilter {
                dimension: "repository".to_owned(),
                values: vec!["org/repo".to_owned()],
            }],
            display_dimensions: vec!["category".to_owned()],
        };
        ValidatedMetricDrilldown {
            fingerprint: selection_fingerprint(Uuid::nil(), &selection)
                .unwrap_or_else(|error| panic!("selection fingerprint must build: {error}")),
            selection,
            from: NaiveDate::from_ymd_opt(2026, 7, 1)
                .unwrap_or_else(|| panic!("valid test start date")),
            to: NaiveDate::from_ymd_opt(2026, 7, 31)
                .unwrap_or_else(|| panic!("valid test end date")),
            limit: 1,
            cursor: None,
            plan,
            snapshot_id: "snapshot".to_owned(),
        }
    }

    #[test]
    fn value_query_binds_filters_and_cursor() {
        let value = input(MetricInputRole::Value, "commit_count");
        let plan = plan(
            ComputationSpec::Sum {
                value: value.clone(),
            },
            vec![EvidenceInput {
                role: MetricInputRole::Value,
                measure_key: value.measure_key,
                presentation: evidence_presentation(
                    "git",
                    "commit_count",
                    EvidenceGranularity::Event,
                ),
            }],
        );
        let mut request = validated(plan);
        request.cursor = Some(CursorKey {
            role: "value".to_owned(),
            metric_date: "2026-07-01".to_owned(),
            observed_at: String::new(),
            source_key: "git".to_owned(),
            measure_key: "commit_count".to_owned(),
            record_id: "abc".to_owned(),
            record_kind: "commit".to_owned(),
            subject_key: String::new(),
        });
        let (sql, params) =
            compile_query(&request).unwrap_or_else(|error| panic!("query must compile: {error}"));
        assert!(sql.contains("insight.git_metric_evidence"));
        assert!(sql.contains("indexOf(evidence.dimensions.1, ?)"));
        assert!(sql.contains("LIMIT 2"));
        assert_eq!(
            params
                .iter()
                .filter(|value| value.as_str() == "repository")
                .count(),
            2
        );
        assert!(params.iter().any(|value| value == "abc"));
    }

    #[test]
    fn ratio_query_uses_named_inputs() {
        let numerator = input(MetricInputRole::Numerator, "focus_hours");
        let denominator = input(MetricInputRole::Denominator, "work_hours");
        let plan = plan(
            ComputationSpec::Ratio {
                numerator: numerator.clone(),
                denominator: denominator.clone(),
                scale: 100.0,
            },
            vec![
                EvidenceInput {
                    role: MetricInputRole::Numerator,
                    measure_key: numerator.measure_key,
                    presentation: EvidencePresentation {
                        detail_keys: &[],
                        show_value: true,
                    },
                },
                EvidenceInput {
                    role: MetricInputRole::Denominator,
                    measure_key: denominator.measure_key,
                    presentation: EvidencePresentation {
                        detail_keys: &[],
                        show_value: true,
                    },
                },
            ],
        );
        let request = validated(plan);
        let (sql, params) =
            compile_query(&request).unwrap_or_else(|error| panic!("query must compile: {error}"));
        assert!(sql.contains("sumIf"));
        assert!(sql.contains("daily_ratio"));
        assert!(params.iter().any(|value| value == "focus_hours"));
        assert!(params.iter().any(|value| value == "work_hours"));
    }

    #[test]
    fn event_presentation_projects_human_fields_and_dimensions() {
        let value = input(MetricInputRole::Value, "commit_count");
        let plan = plan(
            ComputationSpec::Sum {
                value: value.clone(),
            },
            vec![EvidenceInput {
                role: MetricInputRole::Value,
                measure_key: value.measure_key,
                presentation: evidence_presentation(
                    "git",
                    "commit_count",
                    EvidenceGranularity::Event,
                ),
            }],
        );
        let (columns, rows) = presentation(&[row()], &plan, &[], &["category".to_owned()])
            .unwrap_or_else(|error| panic!("presentation must build: {error}"));
        assert_eq!(
            columns
                .iter()
                .map(|column| column.key.as_str())
                .collect::<Vec<_>>(),
            [
                "ref",
                "title",
                "repository",
                "author",
                "category",
                "lines_added",
                "lines_removed",
                "date"
            ]
        );
        assert_eq!(rows[0].values["category"], "code");
        assert_eq!(rows[0].values["lines_added"], 12.0);
    }

    #[test]
    fn ratio_presentation_names_numerator_and_denominator() {
        let numerator = input(MetricInputRole::Numerator, "focus_hours");
        let denominator = input(MetricInputRole::Denominator, "work_hours");
        let plan = plan(
            ComputationSpec::Ratio {
                numerator: numerator.clone(),
                denominator: denominator.clone(),
                scale: 100.0,
            },
            vec![
                EvidenceInput {
                    role: MetricInputRole::Numerator,
                    measure_key: numerator.measure_key,
                    presentation: EvidencePresentation {
                        detail_keys: &[],
                        show_value: true,
                    },
                },
                EvidenceInput {
                    role: MetricInputRole::Denominator,
                    measure_key: denominator.measure_key,
                    presentation: EvidencePresentation {
                        detail_keys: &[],
                        show_value: true,
                    },
                },
            ],
        );
        let mut ratio_row = row();
        ratio_row.numerator = Some(6.0);
        ratio_row.denominator = Some(8.0);
        ratio_row.details = serde_json::json!({});
        let (columns, rows) = presentation(&[ratio_row], &plan, &[], &[])
            .unwrap_or_else(|error| panic!("ratio presentation must build: {error}"));
        assert_eq!(columns[1].label, "Focus hours");
        assert_eq!(columns[2].label, "Work hours");
        assert_eq!(rows[0].values["numerator"], 6.0);
        assert_eq!(rows[0].values["denominator"], 8.0);
    }

    #[test]
    fn response_pages_with_snapshot_bound_cursor() {
        let value = input(MetricInputRole::Value, "commit_count");
        let plan = plan(
            ComputationSpec::Sum {
                value: value.clone(),
            },
            vec![EvidenceInput {
                role: MetricInputRole::Value,
                measure_key: value.measure_key,
                presentation: evidence_presentation(
                    "git",
                    "commit_count",
                    EvidenceGranularity::Event,
                ),
            }],
        );
        let request = validated(plan);
        let response = build_response(&request, vec![row(), row()])
            .unwrap_or_else(|error| panic!("response must build: {error}"));
        let cursor = response
            .next_cursor
            .unwrap_or_else(|| panic!("response must include a next cursor"));
        let envelope =
            decode_cursor(&cursor).unwrap_or_else(|error| panic!("cursor must decode: {error}"));
        assert_eq!(response.rows.len(), 1);
        assert_eq!(envelope.snapshot_id, "snapshot");
        assert_eq!(envelope.fingerprint, request.fingerprint);
        assert!(decode_cursor("invalid").is_err());
    }

    #[test]
    fn filters_and_display_dimensions_are_normalized() {
        let definition = definition(
            ComputationSpec::Sum {
                value: input(MetricInputRole::Value, "commit_count"),
            },
            &["repository", "category"],
        );
        let filters = normalize_filters(
            &definition,
            vec![MetricDrilldownFilter {
                dimension: " repository ".to_owned(),
                values: vec!["b".to_owned(), "a".to_owned(), "a".to_owned()],
            }],
        )
        .unwrap_or_else(|error| panic!("filter value must normalize: {error}"));
        assert_eq!(filters[0].values, ["a", "b"]);
        assert_eq!(
            normalize_display_dimensions(
                &definition,
                vec!["category".to_owned(), "category".to_owned()]
            )
            .unwrap_or_else(|error| panic!("display dimensions must normalize: {error}")),
            ["category"]
        );
        assert!(
            normalize_filters(
                &definition,
                vec![MetricDrilldownFilter {
                    dimension: "unknown".to_owned(),
                    values: vec!["value".to_owned()],
                }]
            )
            .is_err()
        );
        assert!(normalize_display_dimensions(&definition, vec!["unknown".to_owned()]).is_err());
    }

    #[test]
    fn presentation_rejects_invalid_warehouse_json() {
        let value = input(MetricInputRole::Value, "commit_count");
        let plan = plan(
            ComputationSpec::Sum {
                value: value.clone(),
            },
            vec![EvidenceInput {
                role: MetricInputRole::Value,
                measure_key: value.measure_key,
                presentation: evidence_presentation(
                    "git",
                    "commit_count",
                    EvidenceGranularity::Event,
                ),
            }],
        );
        let mut invalid_details = row();
        invalid_details.details = serde_json::json!("invalid");
        assert!(presentation(&[invalid_details], &plan, &[], &[]).is_err());
        let mut invalid_dimensions = row();
        invalid_dimensions.dimensions_json = "invalid".to_owned();
        assert!(presentation(&[invalid_dimensions], &plan, &[], &[]).is_err());
    }

    #[test]
    fn evidence_presentations_cover_domain_shapes() {
        assert!(!evidence_presentation("git", "pr_merged", EvidenceGranularity::Event).show_value);
        assert!(
            evidence_presentation("git", "pr_cycle_hours", EvidenceGranularity::Event).show_value
        );
        assert!(
            evidence_presentation(
                "task",
                "average_slip",
                EvidenceGranularity::DerivedPopulation
            )
            .detail_keys
            .is_empty()
        );
        assert!(evidence_presentation("task", "custom", EvidenceGranularity::Event).show_value);
        assert!(
            !evidence_presentation("wiki", "pages_created", EvidenceGranularity::Event).show_value
        );
        assert!(
            evidence_presentation("collab", "messages", EvidenceGranularity::SourceSummary)
                .show_value
        );
    }
}
