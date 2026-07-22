use std::collections::{BTreeMap, HashMap};

use sea_orm::{ConnectionTrait, DatabaseConnection, FromQueryResult, Statement, Value};
use toolkit_canonical_errors::CanonicalError;
use uuid::Uuid;

use crate::api::error::MetricError;
use crate::domain::metric_definitions::error_code::{MetricSchemaErrorCode, SchemaStatus};

use crate::domain::metric_definitions::definition::{
    ComputationSpec, MetricBase, MetricComputation, MetricDefinition, MetricDirection,
    MetricFormat, MetricInput, MetricInputRole, ObservationRelation, SourceKind, ValueTransform,
};

#[derive(Debug, FromQueryResult)]
struct DefinitionRow {
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
    entity_type: String,
    computation_type: String,
    scale: Option<f64>,
    transform_multiplier: Option<f64>,
    transform_offset: Option<f64>,
    transform_clamp_min: Option<f64>,
    transform_clamp_max: Option<f64>,
    peer_cohort_key: Option<String>,
    definition_enabled: bool,
    definition_schema_status: String,
}

#[derive(Debug, FromQueryResult)]
struct InputRow {
    metric_definition_id: Uuid,
    input_role: String,
    measure_key: String,
    measure_enabled: bool,
    measure_schema_status: String,
    source_key: String,
    source_kind: String,
    source_ref: String,
    source_enabled: bool,
    source_schema_status: String,
}

#[derive(Debug, FromQueryResult)]
struct DimensionRow {
    metric_definition_id: Uuid,
    dimension_key: String,
}

#[derive(Debug)]
enum ClassifiedInputs {
    Available(Vec<MetricInput>),
    Unavailable,
    Corrupt,
}

#[derive(Debug, Clone)]
pub struct MetricDefinitionValidationSpec {
    pub definition_id: Uuid,
    pub metric_key: String,
    pub entity_type: String,
    pub inputs: Vec<MetricInput>,
    pub dimensions: Vec<String>,
}

#[derive(Debug, FromQueryResult)]
struct ValidationDefinitionRow {
    definition_id: Uuid,
    metric_key: String,
    entity_type: String,
}

pub async fn load_definitions(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    metric_keys: &[String],
) -> Result<HashMap<String, MetricDefinition>, CanonicalError> {
    if metric_keys.is_empty() {
        return Ok(HashMap::new());
    }

    let rows = fetch_definition_rows(db, tenant_id, metric_keys)
        .await
        .map_err(|error| db_error(&error))?;
    let all_ids = rows.iter().map(|row| row.definition_id).collect::<Vec<_>>();
    let input_rows = fetch_input_rows(db, &all_ids)
        .await
        .map_err(|error| db_error(&error))?;
    let inputs = classify_inputs(input_rows);
    let dimensions = fetch_dimensions(db, &all_ids)
        .await
        .map_err(|error| db_error(&error))?;

    let mut definitions = HashMap::new();
    for (metric_key, candidates) in group_by_key(rows) {
        let Some(row) = select_available_row(&metric_key, candidates, &inputs)? else {
            continue;
        };
        let definition_id = row.definition_id;
        let row_inputs: &[MetricInput] = match inputs.get(&definition_id) {
            Some(ClassifiedInputs::Available(row_inputs)) => row_inputs,
            Some(ClassifiedInputs::Unavailable | ClassifiedInputs::Corrupt) | None => &[],
        };
        let definition = build_definition(
            &row,
            row_inputs,
            dimensions.get(&definition_id).cloned().unwrap_or_default(),
        )?;
        definitions.insert(metric_key, definition);
    }

    for key in metric_keys {
        if !definitions.contains_key(key) {
            return Err(unavailable(key));
        }
    }

    Ok(definitions)
}

pub async fn managed_definition_validation_specs(
    db: &DatabaseConnection,
    source_id: Uuid,
) -> Result<Vec<MetricDefinitionValidationSpec>, sea_orm::DbErr> {
    let rows = fetch_validation_definition_rows(db, source_id).await?;
    let definition_ids = rows.iter().map(|row| row.definition_id).collect::<Vec<_>>();
    let inputs = classify_inputs(fetch_input_rows(db, &definition_ids).await?);
    let dimensions = fetch_dimensions(db, &definition_ids).await?;

    Ok(rows
        .into_iter()
        .filter_map(|row| {
            let definition_id = row.definition_id;
            match inputs.get(&definition_id) {
                Some(ClassifiedInputs::Available(row_inputs)) => {
                    Some(MetricDefinitionValidationSpec {
                        definition_id,
                        metric_key: row.metric_key,
                        entity_type: row.entity_type,
                        inputs: row_inputs.clone(),
                        dimensions: dimensions.get(&definition_id).cloned().unwrap_or_default(),
                    })
                }
                Some(ClassifiedInputs::Unavailable | ClassifiedInputs::Corrupt) | None => None,
            }
        })
        .collect())
}

async fn fetch_validation_definition_rows(
    db: &DatabaseConnection,
    source_id: Uuid,
) -> Result<Vec<ValidationDefinitionRow>, sea_orm::DbErr> {
    ValidationDefinitionRow::find_by_statement(Statement::from_sql_and_values(
        db.get_database_backend(),
        "SELECT DISTINCT \
            d.id AS definition_id, \
            d.metric_key AS metric_key, \
            d.entity_type AS entity_type \
         FROM metric_definitions d \
         INNER JOIN metric_definition_inputs i ON i.metric_definition_id = d.id \
         INNER JOIN metric_source_measures m ON m.id = i.source_measure_id \
         WHERE d.is_enabled = TRUE \
           AND m.is_enabled = TRUE \
           AND m.source_id = ? \
         ORDER BY d.metric_key",
        [uuid_value(source_id)],
    ))
    .all(db)
    .await
}

async fn fetch_definition_rows(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    metric_keys: &[String],
) -> Result<Vec<DefinitionRow>, sea_orm::DbErr> {
    let placeholders = vec!["?"; metric_keys.len()].join(", ");
    let sql = format!(
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
            d.entity_type AS entity_type, \
            d.computation_type AS computation_type, \
            CAST(d.scale AS DOUBLE) AS scale, \
            CAST(d.transform_multiplier AS DOUBLE) AS transform_multiplier, \
            CAST(d.transform_offset AS DOUBLE) AS transform_offset, \
            CAST(d.transform_clamp_min AS DOUBLE) AS transform_clamp_min, \
            CAST(d.transform_clamp_max AS DOUBLE) AS transform_clamp_max, \
            d.peer_cohort_key AS peer_cohort_key, \
            d.is_enabled AS definition_enabled, \
            d.schema_status AS definition_schema_status \
         FROM metric_definitions d \
         WHERE d.metric_key IN ({placeholders}) \
           AND (d.tenant_id IS NULL OR d.tenant_id = ?)"
    );

    let mut values = metric_keys.iter().map(Value::from).collect::<Vec<_>>();
    values.push(Value::Bytes(Some(Box::new(tenant_id.as_bytes().to_vec()))));

    DefinitionRow::find_by_statement(Statement::from_sql_and_values(
        db.get_database_backend(),
        sql,
        values,
    ))
    .all(db)
    .await
}

async fn fetch_input_rows(
    db: &DatabaseConnection,
    definition_ids: &[Uuid],
) -> Result<Vec<InputRow>, sea_orm::DbErr> {
    if definition_ids.is_empty() {
        return Ok(Vec::new());
    }

    let placeholders = vec!["?"; definition_ids.len()].join(", ");
    let sql = format!(
        "SELECT \
            i.metric_definition_id AS metric_definition_id, \
            i.input_role AS input_role, \
            m.measure_key AS measure_key, \
            m.is_enabled AS measure_enabled, \
            m.schema_status AS measure_schema_status, \
            s.source_key AS source_key, \
            s.source_kind AS source_kind, \
            s.source_ref AS source_ref, \
            s.is_enabled AS source_enabled, \
            s.schema_status AS source_schema_status \
         FROM metric_definition_inputs i \
         INNER JOIN metric_source_measures m ON m.id = i.source_measure_id \
         INNER JOIN metric_sources s ON s.id = m.source_id \
         WHERE i.metric_definition_id IN ({placeholders}) \
         ORDER BY i.metric_definition_id, i.input_role, m.measure_key"
    );
    let values = definition_ids
        .iter()
        .map(|id| Value::Bytes(Some(Box::new(id.as_bytes().to_vec()))))
        .collect::<Vec<_>>();

    InputRow::find_by_statement(Statement::from_sql_and_values(
        db.get_database_backend(),
        sql,
        values,
    ))
    .all(db)
    .await
}

fn classify_inputs(rows: Vec<InputRow>) -> HashMap<Uuid, ClassifiedInputs> {
    let mut out: HashMap<Uuid, ClassifiedInputs> = HashMap::new();
    for row in rows {
        let entry = out
            .entry(row.metric_definition_id)
            .or_insert_with(|| ClassifiedInputs::Available(Vec::new()));

        // Corrupt config must stay loud regardless of row order: it wins
        // over Unavailable, which wins over Available.
        let role = MetricInputRole::from_db(&row.input_role);
        let kind = SourceKind::from_db(&row.source_kind);
        let observation_relation = ObservationRelation::parse(&row.source_ref);
        let parsed = match (role, kind, observation_relation) {
            (Some(role), Some(SourceKind::ManagedObservation), Some(observation_relation)) => {
                Some((role, observation_relation))
            }
            (Some(_), Some(SourceKind::CustomObservationSql), _) => {
                if !matches!(entry, ClassifiedInputs::Corrupt) {
                    *entry = ClassifiedInputs::Unavailable;
                }
                continue;
            }
            _ => None,
        };
        let Some((role, observation_relation)) = parsed else {
            tracing::error!(
                input_role = %row.input_role,
                source_ref = %row.source_ref,
                source_kind = %row.source_kind,
                "corrupt metric definition input"
            );
            *entry = ClassifiedInputs::Corrupt;
            continue;
        };
        if matches!(entry, ClassifiedInputs::Corrupt) {
            continue;
        }

        if !row.measure_enabled
            || !row.source_enabled
            || schema_status_blocks(&row.measure_schema_status)
            || schema_status_blocks(&row.source_schema_status)
        {
            *entry = ClassifiedInputs::Unavailable;
            continue;
        }

        if let ClassifiedInputs::Available(inputs) = entry {
            inputs.push(MetricInput {
                role,
                observation_relation,
                source_key: row.source_key,
                measure_key: row.measure_key,
            });
        }
    }
    out
}

fn schema_status_blocks(status: &str) -> bool {
    !matches!(
        SchemaStatus::from_db(status),
        Some(SchemaStatus::Ok | SchemaStatus::Unchecked)
    )
}

fn group_by_key(rows: Vec<DefinitionRow>) -> BTreeMap<String, Vec<DefinitionRow>> {
    let mut grouped: BTreeMap<String, Vec<DefinitionRow>> = BTreeMap::new();
    for row in rows {
        grouped.entry(row.metric_key.clone()).or_default().push(row);
    }
    grouped
}

fn select_available_row(
    metric_key: &str,
    rows: Vec<DefinitionRow>,
    inputs: &HashMap<Uuid, ClassifiedInputs>,
) -> Result<Option<DefinitionRow>, CanonicalError> {
    let tenant_rows = rows.iter().filter(|row| row.tenant_id.is_some()).count();
    if tenant_rows > 1 {
        return Err(config_error(&format!(
            "multiple tenant metric definitions for {metric_key}"
        )));
    }
    let product_rows = rows.iter().filter(|row| row.tenant_id.is_none()).count();
    if product_rows > 1 {
        return Err(config_error(&format!(
            "multiple product metric definitions for {metric_key}"
        )));
    }

    let mut candidates = rows;
    candidates.sort_by_key(|row| row.tenant_id.is_none());

    for row in candidates {
        if matches!(
            inputs.get(&row.definition_id),
            Some(ClassifiedInputs::Corrupt)
        ) {
            return Err(config_error(&format!(
                "corrupt inputs for metric definition {metric_key}"
            )));
        }
        let inputs_available = !matches!(
            inputs.get(&row.definition_id),
            Some(ClassifiedInputs::Unavailable)
        );
        if row.definition_enabled
            && !schema_status_blocks(&row.definition_schema_status)
            && inputs_available
        {
            return Ok(Some(row));
        }
    }
    Ok(None)
}

async fn fetch_dimensions(
    db: &DatabaseConnection,
    definition_ids: &[Uuid],
) -> Result<HashMap<Uuid, Vec<String>>, sea_orm::DbErr> {
    if definition_ids.is_empty() {
        return Ok(HashMap::new());
    }

    let placeholders = vec!["?"; definition_ids.len()].join(", ");
    let sql = format!(
        "SELECT \
            d.metric_definition_id AS metric_definition_id, \
            s.dimension_key AS dimension_key \
         FROM metric_definition_dimensions d \
         INNER JOIN metric_source_dimensions s ON s.id = d.source_dimension_id \
         WHERE d.metric_definition_id IN ({placeholders}) \
         ORDER BY d.metric_definition_id, d.display_order, s.dimension_key"
    );
    let values = definition_ids
        .iter()
        .map(|id| Value::Bytes(Some(Box::new(id.as_bytes().to_vec()))))
        .collect::<Vec<_>>();

    DimensionRow::find_by_statement(Statement::from_sql_and_values(
        db.get_database_backend(),
        sql,
        values,
    ))
    .all(db)
    .await
    .map(|rows| {
        let mut out: HashMap<Uuid, Vec<String>> = HashMap::new();
        for row in rows {
            out.entry(row.metric_definition_id)
                .or_default()
                .push(row.dimension_key);
        }
        out
    })
}

fn build_definition(
    row: &DefinitionRow,
    inputs: &[MetricInput],
    allowed_dimensions: Vec<String>,
) -> Result<MetricDefinition, CanonicalError> {
    let computation = MetricComputation::from_db(&row.computation_type).ok_or_else(|| {
        config_error(&format!(
            "unknown metric computation for {}",
            row.metric_key
        ))
    })?;
    let base = build_base(row, allowed_dimensions)?;

    let spec = match computation {
        MetricComputation::Sum => ComputationSpec::Sum {
            value: one_input(&row.metric_key, inputs, MetricInputRole::Value)?,
        },
        MetricComputation::Ratio => {
            let numerator = one_input(&row.metric_key, inputs, MetricInputRole::Numerator)?;
            let denominator = one_input(&row.metric_key, inputs, MetricInputRole::Denominator)?;
            if numerator.observation_relation != denominator.observation_relation
                || numerator.source_key != denominator.source_key
            {
                return Err(config_error(&format!(
                    "ratio inputs must share one source for {}",
                    row.metric_key
                )));
            }
            let scale = row.scale.ok_or_else(|| {
                config_error(&format!("missing ratio scale for {}", row.metric_key))
            })?;
            ComputationSpec::Ratio {
                numerator,
                denominator,
                scale,
            }
        }
        MetricComputation::Median => ComputationSpec::Median {
            value: one_input(&row.metric_key, inputs, MetricInputRole::Value)?,
        },
        MetricComputation::DistinctCount => ComputationSpec::DistinctCount {
            value: one_input(&row.metric_key, inputs, MetricInputRole::Value)?,
        },
    };

    let transform = ValueTransform {
        multiplier: row.transform_multiplier,
        offset: row.transform_offset,
        clamp_min: row.transform_clamp_min,
        clamp_max: row.transform_clamp_max,
    };
    let transform = (!transform.is_identity()).then_some(transform);

    Ok(MetricDefinition {
        base,
        spec,
        transform,
    })
}

fn build_base(
    row: &DefinitionRow,
    allowed_dimensions: Vec<String>,
) -> Result<MetricBase, CanonicalError> {
    let format = MetricFormat::from_db(&row.format)
        .ok_or_else(|| config_error(&format!("unknown metric format for {}", row.metric_key)))?;
    let direction = MetricDirection::from_db(&row.direction)
        .ok_or_else(|| config_error(&format!("unknown metric direction for {}", row.metric_key)))?;

    Ok(MetricBase {
        key: row.metric_key.clone(),
        label: row.label.clone(),
        short_label: row.short_label.clone(),
        description: row.description.clone(),
        explanation: row.explanation.clone(),
        entity_type: row.entity_type.clone(),
        format,
        unit: row.unit.clone(),
        direction,
        peer_cohort_key: row.peer_cohort_key.clone(),
        allowed_dimensions,
    })
}

fn one_input(
    metric_key: &str,
    inputs: &[MetricInput],
    role: MetricInputRole,
) -> Result<MetricInput, CanonicalError> {
    let matches = inputs
        .iter()
        .filter(|input| input.role == role)
        .cloned()
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [input] => Ok(input.clone()),
        [] => Err(config_error(&format!(
            "missing {role:?} input for {metric_key}"
        ))),
        _ => Err(config_error(&format!(
            "duplicate {role:?} inputs for {metric_key}"
        ))),
    }
}

pub async fn all_managed_sources(
    db: &DatabaseConnection,
) -> Result<Vec<(Uuid, String, String)>, sea_orm::DbErr> {
    #[derive(FromQueryResult)]
    struct Row {
        id: Uuid,
        source_kind: String,
        source_ref: String,
    }

    Row::find_by_statement(Statement::from_string(
        db.get_database_backend(),
        "SELECT id, source_kind, source_ref \
         FROM metric_sources \
         WHERE is_enabled = TRUE",
    ))
    .all(db)
    .await
    .map(|rows| {
        rows.into_iter()
            .map(|row| (row.id, row.source_kind, row.source_ref))
            .collect()
    })
}

// `updated_at = updated_at` in the status writers below pins the column so
// ON UPDATE CURRENT_TIMESTAMP(3) does not fire: updated_at tracks config
// edits, not validator sweeps.
pub async fn update_source_status(
    db: &DatabaseConnection,
    source_id: Uuid,
    status: SchemaStatus,
    error_code: Option<MetricSchemaErrorCode>,
) -> Result<(), sea_orm::DbErr> {
    db.execute(Statement::from_sql_and_values(
        db.get_database_backend(),
        "UPDATE metric_sources \
         SET schema_status = ?, \
             schema_checked_at = CURRENT_TIMESTAMP(3), \
             schema_error_code = ?, \
             updated_at = updated_at \
         WHERE id = ?",
        [
            Value::from(status.as_db()),
            match error_code {
                Some(code) => Value::from(code.as_db()),
                None => Value::String(None),
            },
            Value::Bytes(Some(Box::new(source_id.as_bytes().to_vec()))),
        ],
    ))
    .await?;
    Ok(())
}

pub async fn update_definitions_for_source_status(
    db: &DatabaseConnection,
    source_id: Uuid,
    status: SchemaStatus,
    error_code: Option<MetricSchemaErrorCode>,
) -> Result<(), sea_orm::DbErr> {
    db.execute(Statement::from_sql_and_values(
        db.get_database_backend(),
        "UPDATE metric_definitions \
         SET schema_status = ?, \
             schema_checked_at = CURRENT_TIMESTAMP(3), \
             schema_error_code = ?, \
             updated_at = updated_at \
         WHERE id IN ( \
             SELECT metric_definition_id \
             FROM metric_definition_inputs i \
             INNER JOIN metric_source_measures m ON m.id = i.source_measure_id \
             WHERE m.source_id = ? \
         )",
        [
            Value::from(status.as_db()),
            match error_code {
                Some(code) => Value::from(code.as_db()),
                None => Value::String(None),
            },
            Value::Bytes(Some(Box::new(source_id.as_bytes().to_vec()))),
        ],
    ))
    .await?;
    Ok(())
}

pub async fn update_definition_status(
    db: &DatabaseConnection,
    definition_id: Uuid,
    status: SchemaStatus,
    error_code: Option<MetricSchemaErrorCode>,
) -> Result<(), sea_orm::DbErr> {
    db.execute(Statement::from_sql_and_values(
        db.get_database_backend(),
        "UPDATE metric_definitions \
         SET schema_status = ?, \
             schema_checked_at = CURRENT_TIMESTAMP(3), \
             schema_error_code = ?, \
             updated_at = updated_at \
         WHERE id = ?",
        [
            Value::from(status.as_db()),
            match error_code {
                Some(code) => Value::from(code.as_db()),
                None => Value::String(None),
            },
            uuid_value(definition_id),
        ],
    ))
    .await?;
    Ok(())
}

fn uuid_value(value: Uuid) -> Value {
    Value::Bytes(Some(Box::new(value.as_bytes().to_vec())))
}

fn unavailable(metric_key: &str) -> CanonicalError {
    MetricError::invalid_argument()
        .with_field_violation(
            "metrics.metric_key",
            format!("unknown or unavailable metric key: {metric_key}"),
            "UNAVAILABLE",
        )
        .create()
}

fn config_error(message: &str) -> CanonicalError {
    tracing::error!(message = %message, "metric definition configuration error");
    CanonicalError::internal("metric definition configuration error").create()
}

fn db_error(error: &sea_orm::DbErr) -> CanonicalError {
    tracing::error!(error = %error, "metric definition database query failed");
    CanonicalError::internal("metric definition lookup failed").create()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn definition_row(
        metric_key: &str,
        tenant: Option<Uuid>,
        enabled: bool,
        schema_status: &str,
    ) -> DefinitionRow {
        DefinitionRow {
            definition_id: Uuid::now_v7(),
            tenant_id: tenant,
            metric_key: metric_key.to_owned(),
            label: "Label".to_owned(),
            short_label: None,
            description: None,
            explanation: None,
            unit: None,
            format: "integer".to_owned(),
            direction: "neutral".to_owned(),
            entity_type: "person".to_owned(),
            computation_type: "sum".to_owned(),
            scale: None,
            transform_multiplier: None,
            transform_offset: None,
            transform_clamp_min: None,
            transform_clamp_max: None,
            peer_cohort_key: Some("org_unit".to_owned()),
            definition_enabled: enabled,
            definition_schema_status: schema_status.to_owned(),
        }
    }

    fn input_row(definition_id: Uuid, role: &str, enabled: bool, status: &str) -> InputRow {
        InputRow {
            metric_definition_id: definition_id,
            input_role: role.to_owned(),
            measure_key: "accepted_lines".to_owned(),
            measure_enabled: enabled,
            measure_schema_status: status.to_owned(),
            source_key: "ai_usage".to_owned(),
            source_kind: "managed_observation".to_owned(),
            source_ref: "ai_metric_observations".to_owned(),
            source_enabled: true,
            source_schema_status: "ok".to_owned(),
        }
    }

    fn available_inputs(id: Uuid) -> HashMap<Uuid, ClassifiedInputs> {
        classify_inputs(vec![input_row(id, "value", true, "ok")])
    }

    #[test]
    fn selects_tenant_row_over_product() {
        let tenant = Uuid::now_v7();
        let tenant_row = definition_row("ai.x", Some(tenant), true, "ok");
        let product_row = definition_row("ai.x", None, true, "ok");
        let tenant_id = tenant_row.definition_id;
        let mut inputs = available_inputs(tenant_id);
        inputs.extend(available_inputs(product_row.definition_id));

        let Ok(Some(selected)) =
            select_available_row("ai.x", vec![product_row, tenant_row], &inputs)
        else {
            panic!("expected selected row");
        };
        assert_eq!(selected.definition_id, tenant_id);
    }

    #[test]
    fn disabled_tenant_row_falls_back_to_product() {
        let tenant = Uuid::now_v7();
        let tenant_row = definition_row("ai.x", Some(tenant), false, "ok");
        let product_row = definition_row("ai.x", None, true, "ok");
        let product_id = product_row.definition_id;
        let mut inputs = available_inputs(tenant_row.definition_id);
        inputs.extend(available_inputs(product_id));

        let Ok(Some(selected)) =
            select_available_row("ai.x", vec![tenant_row, product_row], &inputs)
        else {
            panic!("expected selected row");
        };
        assert_eq!(selected.definition_id, product_id);
    }

    #[test]
    fn schema_error_tenant_row_falls_back_to_product() {
        let tenant = Uuid::now_v7();
        let tenant_row = definition_row("ai.x", Some(tenant), true, "error");
        let product_row = definition_row("ai.x", None, true, "ok");
        let product_id = product_row.definition_id;
        let mut inputs = available_inputs(tenant_row.definition_id);
        inputs.extend(available_inputs(product_id));

        let Ok(Some(selected)) =
            select_available_row("ai.x", vec![tenant_row, product_row], &inputs)
        else {
            panic!("expected selected row");
        };
        assert_eq!(selected.definition_id, product_id);
    }

    #[test]
    fn unavailable_inputs_fall_back_to_product() {
        let tenant = Uuid::now_v7();
        let tenant_row = definition_row("ai.x", Some(tenant), true, "ok");
        let product_row = definition_row("ai.x", None, true, "ok");
        let product_id = product_row.definition_id;
        let mut inputs = classify_inputs(vec![input_row(
            tenant_row.definition_id,
            "value",
            false,
            "ok",
        )]);
        inputs.extend(available_inputs(product_id));

        let Ok(Some(selected)) =
            select_available_row("ai.x", vec![tenant_row, product_row], &inputs)
        else {
            panic!("expected selected row");
        };
        assert_eq!(selected.definition_id, product_id);
    }

    #[test]
    fn no_available_row_yields_none() {
        let tenant = Uuid::now_v7();
        let tenant_row = definition_row("ai.x", Some(tenant), false, "ok");
        let product_row = definition_row("ai.x", None, true, "error");
        let mut inputs = available_inputs(tenant_row.definition_id);
        inputs.extend(available_inputs(product_row.definition_id));

        let Ok(selected) = select_available_row("ai.x", vec![tenant_row, product_row], &inputs)
        else {
            panic!("expected ok selection");
        };
        assert!(selected.is_none());
    }

    #[test]
    fn product_only_selection_works() {
        let product_row = definition_row("ai.x", None, true, "ok");
        let product_id = product_row.definition_id;
        let inputs = available_inputs(product_id);

        let Ok(Some(selected)) = select_available_row("ai.x", vec![product_row], &inputs) else {
            panic!("expected selected row");
        };
        assert_eq!(selected.definition_id, product_id);
    }

    #[test]
    fn duplicate_tenant_rows_are_config_errors() {
        let tenant = Uuid::now_v7();
        let rows = vec![
            definition_row("ai.x", Some(tenant), true, "ok"),
            definition_row("ai.x", Some(tenant), true, "ok"),
        ];
        assert!(select_available_row("ai.x", rows, &HashMap::new()).is_err());
    }

    #[test]
    fn corrupt_inputs_are_config_errors_not_fallback() {
        let tenant = Uuid::now_v7();
        let tenant_row = definition_row("ai.x", Some(tenant), true, "ok");
        let product_row = definition_row("ai.x", None, true, "ok");
        let mut inputs = classify_inputs(vec![InputRow {
            input_role: "nonsense".to_owned(),
            ..input_row(tenant_row.definition_id, "value", true, "ok")
        }]);
        inputs.extend(available_inputs(product_row.definition_id));

        assert!(select_available_row("ai.x", vec![tenant_row, product_row], &inputs).is_err());
    }

    #[test]
    fn classify_corrupt_wins_over_earlier_unavailable_row() {
        let id = Uuid::now_v7();
        let disabled = input_row(id, "value", false, "ok");
        let corrupt = InputRow {
            input_role: "nonsense".to_owned(),
            ..input_row(id, "value", true, "ok")
        };
        let classified = classify_inputs(vec![disabled, corrupt]);
        assert!(matches!(
            classified.get(&id),
            Some(ClassifiedInputs::Corrupt)
        ));
    }

    #[test]
    fn classify_corrupt_is_not_downgraded_by_later_rows() {
        let id = Uuid::now_v7();
        let corrupt = InputRow {
            input_role: "nonsense".to_owned(),
            ..input_row(id, "value", true, "ok")
        };
        let disabled = input_row(id, "value", false, "ok");
        let available = input_row(id, "value", true, "ok");
        let classified = classify_inputs(vec![corrupt, disabled, available]);
        assert!(matches!(
            classified.get(&id),
            Some(ClassifiedInputs::Corrupt)
        ));
    }

    #[test]
    fn classify_corrupt_is_not_downgraded_by_later_custom_sql_row() {
        let id = Uuid::now_v7();
        let corrupt = InputRow {
            input_role: "nonsense".to_owned(),
            ..input_row(id, "value", true, "ok")
        };
        let mut custom = input_row(id, "value", true, "ok");
        custom.source_kind = "custom_observation_sql".to_owned();
        let classified = classify_inputs(vec![corrupt, custom]);
        assert!(matches!(
            classified.get(&id),
            Some(ClassifiedInputs::Corrupt)
        ));
    }

    #[test]
    fn classify_marks_custom_sql_source_unavailable_not_corrupt() {
        let id = Uuid::now_v7();
        let mut row = input_row(id, "value", true, "ok");
        row.source_kind = "custom_observation_sql".to_owned();
        let classified = classify_inputs(vec![row]);
        assert!(matches!(
            classified.get(&id),
            Some(ClassifiedInputs::Unavailable)
        ));
    }

    #[test]
    fn classify_marks_disabled_source_unavailable() {
        let id = Uuid::now_v7();
        let mut row = input_row(id, "value", true, "ok");
        row.source_enabled = false;
        let classified = classify_inputs(vec![row]);
        assert!(matches!(
            classified.get(&id),
            Some(ClassifiedInputs::Unavailable)
        ));
    }

    #[test]
    fn classify_keeps_available_inputs() {
        let id = Uuid::now_v7();
        let classified = classify_inputs(vec![
            input_row(id, "numerator", true, "ok"),
            input_row(id, "denominator", true, "ok"),
        ]);
        match classified.get(&id) {
            Some(ClassifiedInputs::Available(inputs)) => assert_eq!(inputs.len(), 2),
            other => panic!("expected available inputs, got {other:?}"),
        }
    }

    #[test]
    fn one_input_rejects_missing_and_duplicate_roles() {
        let input = MetricInput {
            role: MetricInputRole::Value,
            observation_relation: ObservationRelation::parse("ai_metric_observations")
                .unwrap_or_else(|| panic!("fixture relation must parse")),
            source_key: "ai_usage".to_owned(),
            measure_key: "accepted_lines".to_owned(),
        };
        assert!(one_input("ai.x", &[], MetricInputRole::Value).is_err());
        assert!(one_input("ai.x", std::slice::from_ref(&input), MetricInputRole::Value).is_ok());
        assert!(one_input("ai.x", &[input.clone(), input], MetricInputRole::Value).is_err());
    }

    #[test]
    fn build_base_maps_short_label_and_full_fields() {
        let mut row = definition_row("git.commits", None, true, "ok");
        row.short_label = Some("Commits".to_owned());
        let base = build_base(&row, vec!["repository".to_owned()])
            .unwrap_or_else(|_| panic!("valid row maps to a base"));
        assert_eq!(base.key, "git.commits");
        assert_eq!(base.short_label.as_deref(), Some("Commits"));
        assert_eq!(base.allowed_dimensions, vec!["repository".to_owned()]);
    }
}
