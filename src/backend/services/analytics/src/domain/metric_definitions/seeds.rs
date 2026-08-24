use sea_orm::{ConnectionTrait, DatabaseConnection, DbErr, Statement, TransactionTrait, Value};
use uuid::Uuid;

use crate::domain::metric_definitions::builtin::{
    BuiltinSource, CohortKey, InputSeed, MetricSeed, builtin_metrics, builtin_sources,
};

pub async fn reconcile_builtin_definitions(db: &DatabaseConnection) -> Result<(), DbErr> {
    for builtin_source in builtin_sources() {
        reconcile_source(db, builtin_source).await?;
    }

    // One metric's definition and all its child rows (inputs, dimensions, tags)
    // converge in a single transaction: a mid-way failure never leaves a metric
    // with a partial child set, and a concurrent reconciler on another replica
    // sees the whole prior set or the whole new one, never a delete-in-progress.
    // DESIGN requires builtin upserts to be idempotent and race-safe.
    for metric in builtin_metrics() {
        let txn = db.begin().await?;
        let source_id = fetch_source_id(&txn, &metric.source_key).await?;
        upsert_metric(&txn, metric).await?;
        let metric_id = fetch_metric_id(&txn, &metric.metric_key).await?;
        replace_inputs(&txn, source_id, metric_id, &metric.inputs).await?;
        replace_dimensions(&txn, source_id, metric_id, &metric.dimensions).await?;
        replace_tags(&txn, metric_id, &metric.tags).await?;
        txn.commit().await?;
    }

    disable_missing_builtin_rows(db).await?;
    Ok(())
}

async fn reconcile_source(
    db: &DatabaseConnection,
    builtin_source: &BuiltinSource,
) -> Result<(), DbErr> {
    upsert_source(db, builtin_source).await?;
    let source_id = fetch_source_id(db, &builtin_source.source.key).await?;

    for measure in &builtin_source.measures {
        db.execute(Statement::from_sql_and_values(
            db.get_database_backend(),
            "INSERT INTO metric_source_measures \
                (id, source_id, measure_key, evidence_granularity, is_enabled) \
             VALUES (?, ?, ?, ?, TRUE) \
             ON DUPLICATE KEY UPDATE \
                evidence_granularity = VALUES(evidence_granularity), \
                is_enabled = VALUES(is_enabled)",
            [
                uuid_value(Uuid::now_v7()),
                uuid_value(source_id),
                Value::from(measure.key.as_str()),
                Value::from(measure.evidence_granularity.as_db()),
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
                Value::from(dimension_key.as_str()),
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
            (id, tenant_id, source_key, source_kind, source_ref, evidence_ref, origin, is_enabled) \
         VALUES (?, NULL, ?, ?, ?, ?, 'builtin', TRUE) \
         ON DUPLICATE KEY UPDATE \
            source_kind = VALUES(source_kind), \
            source_ref = VALUES(source_ref), \
            evidence_ref = VALUES(evidence_ref), \
            origin = VALUES(origin), \
            is_enabled = VALUES(is_enabled)",
        [
            uuid_value(Uuid::now_v7()),
            Value::from(builtin_source.source.key.as_str()),
            Value::from(builtin_source.source.kind.as_db()),
            Value::from(builtin_source.source.source_ref.as_str()),
            Value::from(builtin_source.source.evidence_ref.as_str()),
        ],
    ))
    .await?;
    Ok(())
}

async fn upsert_metric(db: &impl ConnectionTrait, metric: &MetricSeed) -> Result<(), DbErr> {
    db.execute(Statement::from_sql_and_values(
        db.get_database_backend(),
        "INSERT INTO metric_definitions \
            (id, tenant_id, metric_key, label, short_label, subject, description, explanation, unit, format, direction, entity_type, \
             computation_type, scale, transform_multiplier, transform_offset, transform_clamp_min, \
             transform_clamp_max, peer_cohort_key, denominator_aggregation, origin, is_enabled) \
         VALUES (?, NULL, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 'builtin', TRUE) \
         ON DUPLICATE KEY UPDATE \
            label = VALUES(label), \
            short_label = VALUES(short_label), \
            subject = VALUES(subject), \
            description = VALUES(description), \
            explanation = VALUES(explanation), \
            unit = VALUES(unit), \
            format = VALUES(format), \
            direction = VALUES(direction), \
            entity_type = VALUES(entity_type), \
            computation_type = VALUES(computation_type), \
            scale = VALUES(scale), \
            transform_multiplier = VALUES(transform_multiplier), \
            transform_offset = VALUES(transform_offset), \
            transform_clamp_min = VALUES(transform_clamp_min), \
            transform_clamp_max = VALUES(transform_clamp_max), \
            peer_cohort_key = VALUES(peer_cohort_key), \
            denominator_aggregation = VALUES(denominator_aggregation), \
            origin = VALUES(origin), \
            is_enabled = VALUES(is_enabled)",
        [
            uuid_value(Uuid::now_v7()),
            Value::from(metric.metric_key.as_str()),
            Value::from(metric.label.as_str()),
            nullable_str(metric.short_label.as_deref()),
            Value::from(metric.subject.as_str()),
            nullable_str(metric.description.as_deref()),
            nullable_str(metric.explanation.as_deref()),
            nullable_str(metric.unit.as_deref()),
            Value::from(metric.format.as_db()),
            Value::from(metric.direction.as_db()),
            Value::from(metric.entity_type.as_db()),
            Value::from(metric.computation.computation().as_db()),
            match metric.computation.scale() {
                Some(scale) => Value::from(scale),
                None => Value::Double(None),
            },
            nullable_f64(metric.transform.and_then(|t| t.multiplier)),
            nullable_f64(metric.transform.and_then(|t| t.offset)),
            nullable_f64(metric.transform.and_then(|t| t.clamp_min)),
            nullable_f64(metric.transform.and_then(|t| t.clamp_max)),
            nullable_str(metric.peer_cohort_key.map(CohortKey::as_db)),
            Value::from(metric.computation.denominator_aggregation().as_db()),
        ],
    ))
    .await?;
    Ok(())
}

async fn replace_inputs(
    db: &impl ConnectionTrait,
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
        let measure_id = fetch_measure_id(db, source_id, &input.measure_key).await?;
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
    db: &impl ConnectionTrait,
    source_id: Uuid,
    metric_id: Uuid,
    dimensions: &[String],
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

async fn replace_tags(
    db: &impl ConnectionTrait,
    metric_id: Uuid,
    tags: &[String],
) -> Result<(), DbErr> {
    db.execute(Statement::from_sql_and_values(
        db.get_database_backend(),
        "DELETE FROM metric_definition_tags WHERE metric_definition_id = ?",
        [uuid_value(metric_id)],
    ))
    .await?;

    for (idx, tag) in tags.iter().enumerate() {
        db.execute(Statement::from_sql_and_values(
            db.get_database_backend(),
            "INSERT INTO metric_definition_tags \
                (id, metric_definition_id, tag, display_order) \
             VALUES (?, ?, ?, ?)",
            [
                uuid_value(Uuid::now_v7()),
                uuid_value(metric_id),
                Value::from(tag.as_str()),
                Value::from(order_value(idx)),
            ],
        ))
        .await?;
    }
    Ok(())
}

async fn disable_missing_builtin_rows(db: &DatabaseConnection) -> Result<(), DbErr> {
    let metric_keys = builtin_metrics()
        .iter()
        .map(|metric| metric.metric_key.as_str())
        .collect::<Vec<_>>();
    disable_missing(
        db,
        "UPDATE metric_definitions SET is_enabled = FALSE \
         WHERE tenant_id IS NULL AND origin = 'builtin' AND is_enabled = TRUE",
        "metric_key",
        &metric_keys,
    )
    .await?;

    let source_keys = builtin_sources()
        .iter()
        .map(|builtin_source| builtin_source.source.key.as_str())
        .collect::<Vec<_>>();
    disable_missing(
        db,
        "UPDATE metric_sources SET is_enabled = FALSE \
         WHERE tenant_id IS NULL AND origin = 'builtin' AND is_enabled = TRUE",
        "source_key",
        &source_keys,
    )
    .await?;

    for builtin_source in builtin_sources() {
        let source_id = fetch_source_id(db, &builtin_source.source.key).await?;
        let measure_keys = builtin_source
            .measures
            .iter()
            .map(|measure| measure.key.as_str())
            .collect::<Vec<_>>();

        let base_sql = "UPDATE metric_source_measures SET is_enabled = FALSE \
                        WHERE source_id = ? AND is_enabled = TRUE";
        let sql = if measure_keys.is_empty() {
            base_sql.to_owned()
        } else {
            let placeholders = vec!["?"; measure_keys.len()].join(", ");
            format!("{base_sql} AND measure_key NOT IN ({placeholders})")
        };

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

async fn fetch_source_id(db: &impl ConnectionTrait, source_key: &str) -> Result<Uuid, DbErr> {
    fetch_uuid(
        db,
        "SELECT id FROM metric_sources WHERE tenant_id IS NULL AND source_key = ?",
        &[Value::from(source_key)],
        source_key,
    )
    .await
}

async fn fetch_measure_id(
    db: &impl ConnectionTrait,
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
    db: &impl ConnectionTrait,
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

async fn fetch_metric_id(db: &impl ConnectionTrait, metric_key: &str) -> Result<Uuid, DbErr> {
    fetch_uuid(
        db,
        "SELECT id FROM metric_definitions WHERE tenant_id IS NULL AND metric_key = ?",
        &[Value::from(metric_key)],
        metric_key,
    )
    .await
}

async fn fetch_uuid(
    db: &impl ConnectionTrait,
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

fn nullable_f64(value: Option<f64>) -> Value {
    match value {
        Some(value) => Value::from(value),
        None => Value::Double(None),
    }
}
