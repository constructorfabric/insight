use sea_orm::{ConnectionTrait, DatabaseConnection, Statement, Value};
use uuid::Uuid;

pub(crate) struct DrilldownFixture {
    pub(crate) tenant_id: Uuid,
    pub(crate) source_id: Uuid,
}

impl DrilldownFixture {
    pub(crate) async fn insert(
        db: &DatabaseConnection,
        metric_keys: &[&str],
        dimensions: &[&str],
    ) -> Result<Self, sea_orm::DbErr> {
        let tenant_id = Uuid::now_v7();
        let source_id = Uuid::now_v7();
        let measure_id = Uuid::now_v7();
        let suffix = tenant_id.simple().to_string();
        let source_key = format!("test_{suffix}");
        let source_ref = format!("test_{suffix}_metric_observations");
        let evidence_ref = format!("test_{suffix}_metric_evidence");

        db.execute_raw(Statement::from_sql_and_values(
            db.get_database_backend(),
            "INSERT INTO metric_sources \
                (id, tenant_id, source_key, source_kind, source_ref, evidence_ref, origin, \
                 schema_status, evidence_schema_status) \
             VALUES (?, ?, ?, 'managed_observation', ?, ?, 'custom', 'ok', 'ok')",
            [
                uuid_value(source_id),
                uuid_value(tenant_id),
                Value::from(source_key),
                Value::from(source_ref),
                Value::from(evidence_ref),
            ],
        ))
        .await?;
        db.execute_raw(Statement::from_sql_and_values(
            db.get_database_backend(),
            "INSERT INTO metric_source_measures \
                (id, source_id, measure_key, evidence_granularity, schema_status) \
             VALUES (?, ?, 'value_count', 'event', 'ok')",
            [uuid_value(measure_id), uuid_value(source_id)],
        ))
        .await?;

        let source_dimensions = insert_dimensions(db, source_id, dimensions).await?;
        insert_definitions(db, tenant_id, measure_id, metric_keys, &source_dimensions).await?;

        db.execute_raw(Statement::from_sql_and_values(
            db.get_database_backend(),
            "UPDATE metric_sources \
             SET schema_status = 'ok', schema_error_code = NULL, \
                 evidence_schema_status = 'ok', evidence_schema_error_code = NULL, \
                 updated_at = updated_at \
             WHERE id = ?",
            [uuid_value(source_id)],
        ))
        .await?;

        Ok(Self {
            tenant_id,
            source_id,
        })
    }

    pub(crate) async fn delete(self, db: &DatabaseConnection) -> Result<(), sea_orm::DbErr> {
        db.execute_raw(Statement::from_sql_and_values(
            db.get_database_backend(),
            "DELETE FROM metric_definitions WHERE tenant_id = ?",
            [uuid_value(self.tenant_id)],
        ))
        .await?;
        db.execute_raw(Statement::from_sql_and_values(
            db.get_database_backend(),
            "DELETE FROM metric_sources WHERE id = ?",
            [uuid_value(self.source_id)],
        ))
        .await?;
        Ok(())
    }

    pub(crate) async fn config_revision(
        &self,
        db: &DatabaseConnection,
    ) -> Result<String, sea_orm::DbErr> {
        let row = db
            .query_one_raw(Statement::from_sql_and_values(
                db.get_database_backend(),
                "SELECT DATE_FORMAT(updated_at, '%Y-%m-%d %H:%i:%s.%f') AS config_revision \
                 FROM metric_sources WHERE id = ?",
                [uuid_value(self.source_id)],
            ))
            .await?
            .ok_or_else(|| sea_orm::DbErr::Custom("test source disappeared".to_owned()))?;
        row.try_get("", "config_revision")
    }

    pub(crate) async fn statuses(
        &self,
        db: &DatabaseConnection,
    ) -> Result<(String, String, Option<String>), sea_orm::DbErr> {
        let row = db
            .query_one_raw(Statement::from_sql_and_values(
                db.get_database_backend(),
                "SELECT schema_status, evidence_schema_status, evidence_schema_error_code \
                 FROM metric_sources WHERE id = ?",
                [uuid_value(self.source_id)],
            ))
            .await?
            .ok_or_else(|| sea_orm::DbErr::Custom("test source disappeared".to_owned()))?;
        Ok((
            row.try_get("", "schema_status")?,
            row.try_get("", "evidence_schema_status")?,
            row.try_get("", "evidence_schema_error_code")?,
        ))
    }
}

async fn insert_dimensions(
    db: &DatabaseConnection,
    source_id: Uuid,
    dimensions: &[&str],
) -> Result<Vec<Uuid>, sea_orm::DbErr> {
    let mut source_dimensions = Vec::with_capacity(dimensions.len());
    for (display_order, dimension) in dimensions.iter().enumerate() {
        let display_order = checked_display_order(display_order)?;
        let dimension_id = Uuid::now_v7();
        db.execute_raw(Statement::from_sql_and_values(
            db.get_database_backend(),
            "INSERT INTO metric_source_dimensions \
                (id, source_id, dimension_key, display_order) \
             VALUES (?, ?, ?, ?)",
            [
                uuid_value(dimension_id),
                uuid_value(source_id),
                Value::from(*dimension),
                Value::from(display_order),
            ],
        ))
        .await?;
        source_dimensions.push(dimension_id);
    }
    Ok(source_dimensions)
}

async fn insert_definitions(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    measure_id: Uuid,
    metric_keys: &[&str],
    source_dimensions: &[Uuid],
) -> Result<(), sea_orm::DbErr> {
    for metric_key in metric_keys {
        let definition_id = Uuid::now_v7();
        db.execute_raw(Statement::from_sql_and_values(
            db.get_database_backend(),
            "INSERT INTO metric_definitions \
                (id, tenant_id, metric_key, label, format, direction, entity_type, \
                 computation_type, origin, schema_status, peer_cohort_key) \
             VALUES (?, ?, ?, ?, 'integer', 'higher_is_better', 'person', \
                     'sum', 'custom', 'ok', 'org_unit')",
            [
                uuid_value(definition_id),
                uuid_value(tenant_id),
                Value::from(*metric_key),
                Value::from(format!("Test {metric_key}")),
            ],
        ))
        .await?;
        db.execute_raw(Statement::from_sql_and_values(
            db.get_database_backend(),
            "INSERT INTO metric_definition_inputs \
                (id, metric_definition_id, input_role, source_measure_id) \
             VALUES (?, ?, 'value', ?)",
            [
                uuid_value(Uuid::now_v7()),
                uuid_value(definition_id),
                uuid_value(measure_id),
            ],
        ))
        .await?;
        insert_definition_dimensions(db, definition_id, source_dimensions).await?;
    }
    Ok(())
}

async fn insert_definition_dimensions(
    db: &DatabaseConnection,
    definition_id: Uuid,
    source_dimensions: &[Uuid],
) -> Result<(), sea_orm::DbErr> {
    for (display_order, dimension_id) in source_dimensions.iter().enumerate() {
        let display_order = checked_display_order(display_order)?;
        db.execute_raw(Statement::from_sql_and_values(
            db.get_database_backend(),
            "INSERT INTO metric_definition_dimensions \
                (id, metric_definition_id, source_dimension_id, display_order) \
             VALUES (?, ?, ?, ?)",
            [
                uuid_value(Uuid::now_v7()),
                uuid_value(definition_id),
                uuid_value(*dimension_id),
                Value::from(display_order),
            ],
        ))
        .await?;
    }
    Ok(())
}

fn checked_display_order(value: usize) -> Result<i32, sea_orm::DbErr> {
    i32::try_from(value).map_err(|_| sea_orm::DbErr::Custom("too many test dimensions".to_owned()))
}

fn uuid_value(value: Uuid) -> Value {
    Value::Bytes(Some(value.as_bytes().to_vec()))
}
