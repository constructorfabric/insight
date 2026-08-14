use sea_orm::{ConnectionTrait, DatabaseConnection, FromQueryResult, Statement, Value};
use toolkit_canonical_errors::CanonicalError;
use uuid::Uuid;

use crate::api::error::MetricError;
use crate::domain::metric_definitions::definition::MetricInputRole;
use crate::domain::metric_definitions::{
    EvidenceGranularity, MetricDefinition, load_definitions_with_ids,
};
use crate::domain::metric_results::{normalize_entity_type, normalize_key, normalize_metric_key};

use super::capability::EvidenceInputRow;
use super::capability::healthy_evidence;
use super::cursor::{
    decode_cursor, evidence_snapshot_id, selection_fingerprint, verify_evidence_snapshot,
};
use super::dto::{
    DEFAULT_PAGE_LIMIT, EvidenceInput, EvidencePlan, MAX_DISPLAY_DIMENSIONS, MAX_EXPORT_ROWS,
    MAX_FILTER_VALUE_BYTES, MAX_FILTER_VALUES, MAX_FILTERS, MAX_PAGE_LIMIT, MAX_PERIOD_DAYS,
    MetricDrilldownEntity, MetricDrilldownExportRequest, MetricDrilldownFilter,
    MetricDrilldownPeriod, MetricDrilldownRequest, MetricDrilldownSelection,
    ValidatedMetricDrilldown,
};

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

use super::error::{
    config_error, db_error, evidence_unavailable, invalid, invalid_error, parse_date,
};
use super::presentation::evidence_presentation;

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

// Canonical person id, like every other person-keyed route since the identity
// cutover; the pre-cutover email shape (and the nil UUID, which parses but is
// never a person) is a loud 400.
//
// INVARIANT: this runs before the visibility gate on both drilldown handlers,
// so it must not reach any backend — an unauthorized caller gets no further.
pub(crate) fn parse_person_entity(
    entity: &MetricDrilldownEntity,
) -> Result<(String, Uuid), CanonicalError> {
    let entity_type = normalize_entity_type(&entity.r#type)?;
    if entity_type != "person" {
        return Err(invalid_error(
            "entity.type",
            "only person entities are supported",
        ));
    }

    let person_id = Uuid::parse_str(entity.id.trim())
        .ok()
        .filter(|id| !id.is_nil())
        .ok_or_else(|| invalid_error("entity.id", "entity.id must be a person UUID"))?;

    Ok((entity_type, person_id))
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
    let (entity_type, person_id) = parse_person_entity(&entity)?;
    let entity_id = person_id.to_string();
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
        tenant_id,
        // The handler overwrites this from config; false is the runtime-wide
        // default (#1967) so tests compile queries in the degraded form.
        enforce_tenant_scope: false,
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
    let Some((relation, source_key)) = healthy_evidence(rows.iter().map(EvidenceInputRow::health))
    else {
        return Err(evidence_unavailable());
    };
    let source_key = source_key.to_owned();
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

fn normalize_filters(
    definition: &MetricDefinition,
    filters: Vec<MetricDrilldownFilter>,
) -> Result<Vec<MetricDrilldownFilter>, CanonicalError> {
    if filters.len() > MAX_FILTERS {
        return invalid("filters", format!("at most {MAX_FILTERS} filters"));
    }
    let mut normalized = Vec::with_capacity(filters.len());
    for filter in filters {
        let dimension = normalize_key("filters.dimension", &filter.dimension)?;
        if definition.allowed_dimension(&dimension).is_none() {
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
        normalized.push(MetricDrilldownFilter { dimension, values });
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
        .map(|dimension| normalize_key("display_dimensions", &dimension))
        .collect::<Result<Vec<_>, _>>()?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::metric_definitions::ComputationSpec;
    use crate::domain::metric_drilldown::test_support::{definition, input};

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
}
