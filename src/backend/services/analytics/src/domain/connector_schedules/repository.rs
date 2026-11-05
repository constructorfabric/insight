use chrono::{SubsecRound, Utc};
use sea_orm::{
    ActiveValue::Set, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, SqlErr,
};
use uuid::Uuid;

use super::{Error, Instance, Revision};
use crate::infra::db::entities::connector_schedules::{ActiveModel, Column, Entity, Model};

#[cfg(test)]
tokio::task_local! {
    pub(crate) static AFTER_WRITE_READ: std::sync::Arc<tokio::sync::Barrier>;
    pub(crate) static AFTER_WRITE_READ_RELEASE: std::sync::Arc<tokio::sync::Notify>;
}

pub(super) async fn read(
    db: &DatabaseConnection,
    instance: &Instance,
) -> Result<Option<Model>, Error> {
    Ok(Entity::find()
        .filter(Column::Connector.eq(instance.connector.as_str()))
        .filter(Column::SourceId.eq(instance.source_id.as_str()))
        .one(db)
        .await?)
}

pub(super) async fn write(
    db: &DatabaseConnection,
    instance: &Instance,
    expected: Revision,
    crons: Vec<String>,
    caller: Uuid,
) -> Result<Option<Model>, Error> {
    let current = read(db, instance).await?;
    #[cfg(test)]
    if let Ok(barrier) = AFTER_WRITE_READ.try_with(std::sync::Arc::clone) {
        barrier.wait().await;
    }
    #[cfg(test)]
    if let Ok(release) = AFTER_WRITE_READ_RELEASE.try_with(std::sync::Arc::clone) {
        release.notified().await;
    }

    let revision = current.as_ref().map_or(0, |row| row.revision);
    if expected.0 != revision {
        return Err(Error::Stale(revision));
    }
    let stored: Vec<String> = current
        .as_ref()
        .map(|row| serde_json::from_value(row.crons.clone()).map_err(Error::StoredCrons))
        .transpose()?
        .unwrap_or_default();
    if stored.len() == crons.len()
        && stored
            .iter()
            .zip(&crons)
            .all(|(stored, requested)| super::validation::normalize(stored) == *requested)
    {
        return Ok(current);
    }
    let revision = expected.0.checked_add(1).ok_or(Error::RevisionExhausted)?;
    let row = Model {
        connector: instance.connector.as_str().to_owned(),
        source_id: instance.source_id.as_str().to_owned(),
        crons: serde_json::json!(crons),
        revision,
        updated_by: caller,
        updated_at: Utc::now().trunc_subsecs(3),
    };
    let active = ActiveModel {
        connector: Set(row.connector.clone()),
        source_id: Set(row.source_id.clone()),
        crons: Set(row.crons.clone()),
        revision: Set(row.revision),
        updated_by: Set(row.updated_by),
        updated_at: Set(row.updated_at),
    };
    if expected.0 == 0 {
        match Entity::insert(active).exec(db).await {
            Ok(_) => return Ok(Some(row)),
            Err(error) if matches!(error.sql_err(), Some(SqlErr::UniqueConstraintViolation(_))) => {
                return Err(stale(db, instance).await?);
            }
            Err(error) => return Err(error.into()),
        }
    }
    let result = Entity::update_many()
        .set(active)
        .filter(Column::Connector.eq(instance.connector.as_str()))
        .filter(Column::SourceId.eq(instance.source_id.as_str()))
        .filter(Column::Revision.eq(expected.0))
        .exec(db)
        .await?;
    if result.rows_affected != 1 {
        return Err(stale(db, instance).await?);
    }
    Ok(Some(row))
}

async fn stale(db: &DatabaseConnection, instance: &Instance) -> Result<Error, Error> {
    let current = read(db, instance).await?;
    Ok(Error::Stale(current.map_or(0, |row| row.revision)))
}

#[cfg(test)]
mod tests;
