//! Full-log read of `persons` for the ClickHouse `identity_persons` sync.
//!
//! Unlike the resolver queries in [`persons_repo`], this needs no windowing —
//! the ClickHouse copy carries the raw log verbatim and all "which observation wins"
//! logic lives on the ClickHouse side (the dbt resolve macro). Plain
//! entity scan, ordered by `id` so the snapshot is deterministic.
//!
//! [`persons_repo`]: super::persons_repo

use async_trait::async_trait;
use sea_orm::{DatabaseConnection, EntityTrait, QueryOrder};
use uuid::Uuid;

use super::entities::persons;
use crate::domain::sync_service::{PersonsLogReader, PersonsLogRow};

/// [`PersonsLogReader`] over the service's MariaDB pool.
pub struct MariaDbPersonsLogReader<'a> {
    db: &'a DatabaseConnection,
}

impl<'a> MariaDbPersonsLogReader<'a> {
    #[must_use]
    pub fn new(db: &'a DatabaseConnection) -> Self {
        Self { db }
    }
}

#[async_trait]
impl PersonsLogReader for MariaDbPersonsLogReader<'_> {
    async fn read_all(&self) -> anyhow::Result<Vec<PersonsLogRow>> {
        let models = persons::Entity::find()
            .order_by_asc(persons::Column::Id)
            .all(self.db)
            .await?;
        models.into_iter().map(map_row).collect()
    }

    async fn latest_id(&self) -> anyhow::Result<Option<u64>> {
        let latest = persons::Entity::find()
            .order_by_desc(persons::Column::Id)
            .one(self.db)
            .await?;
        Ok(latest.map(|m| m.id))
    }
}

fn map_row(m: persons::Model) -> anyhow::Result<PersonsLogRow> {
    Ok(PersonsLogRow {
        id: m.id,
        value_type: m.value_type,
        insight_source_type: m.insight_source_type,
        insight_source_id: uuid16(&m.insight_source_id, "insight_source_id", m.id)?,
        insight_tenant_id: uuid16(&m.insight_tenant_id, "insight_tenant_id", m.id)?,
        value_id: m.value_id,
        value_full_text: m.value_full_text,
        value: m.value,
        value_effective: m.value_effective,
        person_id: uuid16(&m.person_id, "person_id", m.id)?,
        author_person_id: uuid16(&m.author_person_id, "author_person_id", m.id)?,
        // Nullable since migration 009 — copied verbatim, NULL stays NULL.
        reason: m.reason,
        created_at: m.created_at,
    })
}

/// BINARY(16) → `Uuid` (canonical big-endian, as written by `Uuid::as_bytes`).
/// A wrong-length value means a corrupt row — fail the sync loudly rather than
/// copy garbage.
fn uuid16(bytes: &[u8], column: &str, row_id: u64) -> anyhow::Result<Uuid> {
    Uuid::from_slice(bytes)
        .map_err(|e| anyhow::anyhow!("persons.{column} of row id={row_id} is not a UUID: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uuid16_rejects_wrong_length() {
        assert!(uuid16(&[0u8; 15], "person_id", 42).is_err());
        assert!(uuid16(Uuid::from_u128(7).as_bytes(), "person_id", 42).is_ok());
    }
}
