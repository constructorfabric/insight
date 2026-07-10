use sea_orm::{ConnectionTrait, DatabaseConnection, DbErr, Statement, Value};
use uuid::Uuid;

use crate::domain::metric_definitions::builtin::{
    BUILTIN_METRICS, BUILTIN_SOURCES, BuiltinSource, CohortKey, InputSeed, MetricSeed,
};

pub async fn reconcile_builtin_definitions(db: &DatabaseConnection) -> Result<(), DbErr> {
    for builtin_source in BUILTIN_SOURCES {
        reconcile_source(db, builtin_source).await?;
    }

    for metric in BUILTIN_METRICS {
        let source_id = fetch_source_id(db, metric.source_key).await?;
        upsert_metric(db, metric).await?;
        let metric_id = fetch_metric_id(db, metric.metric_key).await?;
        replace_inputs(db, source_id, metric_id, metric.inputs).await?;
        replace_dimensions(db, source_id, metric_id, metric.dimensions).await?;
    }

    disable_missing_builtin_rows(db).await?;
    Ok(())
}

async fn reconcile_source(
    db: &DatabaseConnection,
    builtin_source: &BuiltinSource,
) -> Result<(), DbErr> {
    upsert_source(db, builtin_source).await?;
    let source_id = fetch_source_id(db, builtin_source.source.key).await?;

    for measure_key in builtin_source.measures {
        db.execute(Statement::from_sql_and_values(
            db.get_database_backend(),
            "INSERT INTO metric_source_measures \
                (id, source_id, measure_key, is_enabled) \
             VALUES (?, ?, ?, TRUE) \
             ON DUPLICATE KEY UPDATE \
                is_enabled = VALUES(is_enabled)",
            [
                uuid_value(Uuid::now_v7()),
                uuid_value(source_id),
                Value::from(*measure_key),
            ],
        ))
        .await?;
    }

    for (idx, dimension_key) in builtin_source.dimensions.iter().enumerate() {
        db.execute(Statement::from_sql_and_values(
            db.get_database_backend(),
            "INSERT INTO metric_source_dimensions \
                (id, source_id, dimension_key, display_order) \
             VALUES (?, ?, ?, ?) \
             ON DUPLICATE KEY UPDATE \
                display_order = VALUES(display_order)",
            [
                uuid_value(Uuid::now_v7()),
                uuid_value(source_id),
                Value::from(*dimension_key),
                Value::from(order_value(idx)),
            ],
        ))
        .await?;
    }

    Ok(())
}

async fn upsert_source(
    db: &DatabaseConnection,
    builtin_source: &BuiltinSource,
) -> Result<(), DbErr> {
    db.execute(Statement::from_sql_and_values(
        db.get_database_backend(),
        "INSERT INTO metric_sources \
            (id, tenant_id, source_key, source_kind, source_ref, origin, is_enabled) \
         VALUES (?, NULL, ?, ?, ?, 'builtin', TRUE) \
         ON DUPLICATE KEY UPDATE \
            source_kind = VALUES(source_kind), \
            source_ref = VALUES(source_ref), \
            origin = VALUES(origin), \
            is_enabled = VALUES(is_enabled)",
        [
            uuid_value(Uuid::now_v7()),
            Value::from(builtin_source.source.key),
            Value::from(builtin_source.source.kind.as_db()),
            Value::from(builtin_source.source.source_ref),
        ],
    ))
    .await?;
    Ok(())
}

async fn upsert_metric(db: &DatabaseConnection, metric: &MetricSeed) -> Result<(), DbErr> {
    db.execute(Statement::from_sql_and_values(
        db.get_database_backend(),
        "INSERT INTO metric_definitions \
            (id, tenant_id, metric_key, label, description, explanation, unit, format, direction, entity_type, \
             computation_type, scale, peer_cohort_key, origin, is_enabled) \
         VALUES (?, NULL, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 'builtin', TRUE) \
         ON DUPLICATE KEY UPDATE \
            label = VALUES(label), \
            description = VALUES(description), \
            explanation = VALUES(explanation), \
            unit = VALUES(unit), \
            format = VALUES(format), \
            direction = VALUES(direction), \
            entity_type = VALUES(entity_type), \
            computation_type = VALUES(computation_type), \
            scale = VALUES(scale), \
            peer_cohort_key = VALUES(peer_cohort_key), \
            origin = VALUES(origin), \
            is_enabled = VALUES(is_enabled)",
        [
            uuid_value(Uuid::now_v7()),
            Value::from(metric.metric_key),
            Value::from(metric.label),
            nullable_str(metric.description),
            nullable_str(metric.explanation),
            nullable_str(metric.unit),
            Value::from(metric.format.as_db()),
            Value::from(metric.direction.as_db()),
            Value::from(metric.entity_type.as_db()),
            Value::from(metric.computation.computation().as_db()),
            match metric.computation.scale() {
                Some(scale) => Value::from(scale),
                None => Value::Double(None),
            },
            nullable_str(metric.peer_cohort_key.map(CohortKey::as_db)),
        ],
    ))
    .await?;
    Ok(())
}

async fn replace_inputs(
    db: &DatabaseConnection,
    source_id: Uuid,
    metric_id: Uuid,
    inputs: &[InputSeed],
) -> Result<(), DbErr> {
    db.execute(Statement::from_sql_and_values(
        db.get_database_backend(),
        "DELETE FROM metric_definition_inputs WHERE metric_definition_id = ?",
        [uuid_value(metric_id)],
    ))
    .await?;

    for input in inputs {
        let measure_id = fetch_measure_id(db, source_id, input.measure_key).await?;
        db.execute(Statement::from_sql_and_values(
            db.get_database_backend(),
            "INSERT INTO metric_definition_inputs \
                (id, metric_definition_id, input_role, source_measure_id) \
             VALUES (?, ?, ?, ?)",
            [
                uuid_value(Uuid::now_v7()),
                uuid_value(metric_id),
                Value::from(input.input_role.as_db()),
                uuid_value(measure_id),
            ],
        ))
        .await?;
    }
    Ok(())
}

async fn replace_dimensions(
    db: &DatabaseConnection,
    source_id: Uuid,
    metric_id: Uuid,
    dimensions: &[&str],
) -> Result<(), DbErr> {
    db.execute(Statement::from_sql_and_values(
        db.get_database_backend(),
        "DELETE FROM metric_definition_dimensions WHERE metric_definition_id = ?",
        [uuid_value(metric_id)],
    ))
    .await?;

    for (idx, dimension) in dimensions.iter().enumerate() {
        let dimension_id = fetch_source_dimension_id(db, source_id, dimension).await?;
        db.execute(Statement::from_sql_and_values(
            db.get_database_backend(),
            "INSERT INTO metric_definition_dimensions \
                (id, metric_definition_id, source_dimension_id, display_order) \
             VALUES (?, ?, ?, ?)",
            [
                uuid_value(Uuid::now_v7()),
                uuid_value(metric_id),
                uuid_value(dimension_id),
                Value::from(order_value(idx)),
            ],
        ))
        .await?;
    }
    Ok(())
}

async fn disable_missing_builtin_rows(db: &DatabaseConnection) -> Result<(), DbErr> {
    let metric_keys = BUILTIN_METRICS
        .iter()
        .map(|metric| metric.metric_key)
        .collect::<Vec<_>>();
    disable_missing(
        db,
        "UPDATE metric_definitions SET is_enabled = FALSE \
         WHERE tenant_id IS NULL AND origin = 'builtin' AND is_enabled = TRUE",
        "metric_key",
        &metric_keys,
    )
    .await?;

    let source_keys = BUILTIN_SOURCES
        .iter()
        .map(|builtin_source| builtin_source.source.key)
        .collect::<Vec<_>>();
    disable_missing(
        db,
        "UPDATE metric_sources SET is_enabled = FALSE \
         WHERE tenant_id IS NULL AND origin = 'builtin' AND is_enabled = TRUE",
        "source_key",
        &source_keys,
    )
    .await?;

    for builtin_source in BUILTIN_SOURCES {
        let source_id = fetch_source_id(db, builtin_source.source.key).await?;
        let measure_keys = builtin_source.measures.to_vec();
        let placeholders = vec!["?"; measure_keys.len()].join(", ");
        let sql = format!(
            "UPDATE metric_source_measures SET is_enabled = FALSE \
             WHERE source_id = ? AND is_enabled = TRUE \
               AND measure_key NOT IN ({placeholders})"
        );
        let mut values = vec![uuid_value(source_id)];
        values.extend(measure_keys.iter().map(|key| Value::from(*key)));
        db.execute(Statement::from_sql_and_values(
            db.get_database_backend(),
            sql,
            values,
        ))
        .await?;
    }

    Ok(())
}

async fn disable_missing(
    db: &DatabaseConnection,
    base_sql: &str,
    key_column: &str,
    keys: &[&str],
) -> Result<(), DbErr> {
    let sql = if keys.is_empty() {
        base_sql.to_owned()
    } else {
        let placeholders = vec!["?"; keys.len()].join(", ");
        format!("{base_sql} AND {key_column} NOT IN ({placeholders})")
    };
    let values = keys.iter().map(|key| Value::from(*key)).collect::<Vec<_>>();
    db.execute(Statement::from_sql_and_values(
        db.get_database_backend(),
        sql,
        values,
    ))
    .await?;
    Ok(())
}

async fn fetch_source_id(db: &DatabaseConnection, source_key: &str) -> Result<Uuid, DbErr> {
    fetch_uuid(
        db,
        "SELECT id FROM metric_sources WHERE tenant_id IS NULL AND source_key = ?",
        &[Value::from(source_key)],
        source_key,
    )
    .await
}

async fn fetch_measure_id(
    db: &DatabaseConnection,
    source_id: Uuid,
    measure_key: &str,
) -> Result<Uuid, DbErr> {
    fetch_uuid(
        db,
        "SELECT id FROM metric_source_measures WHERE source_id = ? AND measure_key = ?",
        &[uuid_value(source_id), Value::from(measure_key)],
        measure_key,
    )
    .await
}

async fn fetch_source_dimension_id(
    db: &DatabaseConnection,
    source_id: Uuid,
    dimension_key: &str,
) -> Result<Uuid, DbErr> {
    fetch_uuid(
        db,
        "SELECT id FROM metric_source_dimensions WHERE source_id = ? AND dimension_key = ?",
        &[uuid_value(source_id), Value::from(dimension_key)],
        dimension_key,
    )
    .await
}

async fn fetch_metric_id(db: &DatabaseConnection, metric_key: &str) -> Result<Uuid, DbErr> {
    fetch_uuid(
        db,
        "SELECT id FROM metric_definitions WHERE tenant_id IS NULL AND metric_key = ?",
        &[Value::from(metric_key)],
        metric_key,
    )
    .await
}

async fn fetch_uuid(
    db: &DatabaseConnection,
    sql: &str,
    values: &[Value],
    key: &str,
) -> Result<Uuid, DbErr> {
    let row = db
        .query_one(Statement::from_sql_and_values(
            db.get_database_backend(),
            sql,
            values.to_vec(),
        ))
        .await?
        .ok_or_else(|| DbErr::Custom(format!("missing seeded row for {key}")))?;
    row.try_get("", "id")
}

fn order_value(idx: usize) -> i32 {
    i32::try_from(idx).unwrap_or(i32::MAX)
}

fn uuid_value(id: Uuid) -> Value {
    Value::Bytes(Some(Box::new(id.as_bytes().to_vec())))
}

fn nullable_str(value: Option<&str>) -> Value {
    match value {
        Some(value) => Value::from(value),
        None => Value::String(None),
    }
}
