use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use clickhouse::Row;
use sea_orm::prelude::DateTime;
use serde::Serialize;
use uuid::Uuid;

use crate::domain::sync_service::{IdentityPersonsWriter, PersonsLogRow};
use crate::infra::snapshot_writer::{SnapshotSpec, SnapshotWriter};

const WRITE_TIMEOUT: Duration = Duration::from_mins(5);

const COLUMNS_DDL: &str = r"
    id                  UInt64,
    value_type          String,
    insight_source_type String,
    insight_source_id   UUID,
    insight_tenant_id   UUID,
    value_id            Nullable(String),
    value_full_text     Nullable(String),
    value               Nullable(String),
    value_effective     Nullable(String),
    person_id           UUID,
    author_person_id    UUID,
    reason              Nullable(String),
    created_at          DateTime64(6, 'UTC'),
    _synced_at          DateTime64(3, 'UTC')
";

pub(crate) const SPEC: SnapshotSpec = SnapshotSpec {
    database: "identity",
    target: "identity_persons",
    staging_prefix: "identity_persons_staging_",
    columns_ddl: COLUMNS_DDL,
    order_by: "id",
    watermark_column: "_synced_at",
    log_label: "persons-sync",
};

#[derive(Debug, Row, Serialize)]
struct WireRow {
    id: u64,
    value_type: String,
    insight_source_type: String,
    #[serde(with = "clickhouse::serde::uuid")]
    insight_source_id: Uuid,
    #[serde(with = "clickhouse::serde::uuid")]
    insight_tenant_id: Uuid,
    value_id: Option<String>,
    value_full_text: Option<String>,
    value: Option<String>,
    value_effective: Option<String>,
    #[serde(with = "clickhouse::serde::uuid")]
    person_id: Uuid,
    #[serde(with = "clickhouse::serde::uuid")]
    author_person_id: Uuid,
    reason: Option<String>,
    #[serde(with = "clickhouse::serde::chrono::datetime64::micros")]
    created_at: chrono::DateTime<Utc>,
    #[serde(
        rename = "_synced_at",
        with = "clickhouse::serde::chrono::datetime64::millis"
    )]
    synced_at: chrono::DateTime<Utc>,
}

pub struct ClickHouseIdentityPersonsWriter {
    writer: SnapshotWriter,
}

impl ClickHouseIdentityPersonsWriter {
    #[must_use]
    pub fn connect(url: &str, user: &str, password: &str) -> Self {
        Self {
            writer: SnapshotWriter::connect(url, user, password, SPEC, WRITE_TIMEOUT),
        }
    }
}

#[async_trait]
impl IdentityPersonsWriter for ClickHouseIdentityPersonsWriter {
    async fn replace(&self, rows: &[PersonsLogRow], synced_at: DateTime) -> anyhow::Result<()> {
        let synced_at = synced_at.and_utc();
        let wire: Vec<WireRow> = rows.iter().map(|r| to_wire_row(r, synced_at)).collect();
        self.writer.replace(&wire, synced_at).await
    }
}

fn to_wire_row(r: &PersonsLogRow, synced_at: chrono::DateTime<Utc>) -> WireRow {
    WireRow {
        id: r.id,
        value_type: r.value_type.clone(),
        insight_source_type: r.insight_source_type.clone(),
        insight_source_id: r.insight_source_id,
        insight_tenant_id: r.insight_tenant_id,
        value_id: r.value_id.clone(),
        value_full_text: r.value_full_text.clone(),
        value: r.value.clone(),
        value_effective: r.value_effective.clone(),
        person_id: r.person_id,
        author_person_id: r.author_person_id,
        reason: r.reason.clone(),
        // MariaDB `TIMESTAMP(6)` comes back naive; the pool session runs in
        // UTC, so re-attaching Utc is a re-labeling, not a conversion.
        created_at: r.created_at.and_utc(),
        synced_at,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::sync_service::PersonsLogRow;

    #[test]
    fn maps_log_row_preserving_micros_and_nulls() -> anyhow::Result<()> {
        let created =
            DateTime::parse_from_str("2026-07-29 12:34:56.123456", "%Y-%m-%d %H:%M:%S%.f")?;
        let row = PersonsLogRow {
            id: 9,
            value_type: "email".to_owned(),
            insight_source_type: "bamboohr".to_owned(),
            insight_source_id: Uuid::from_u128(1),
            insight_tenant_id: Uuid::from_u128(2),
            value_id: Some("a@x.com".to_owned()),
            value_full_text: None,
            value: None,
            value_effective: Some("a@x.com".to_owned()),
            person_id: Uuid::from_u128(3),
            author_person_id: Uuid::from_u128(4),
            reason: None,
            created_at: created,
        };
        let synced =
            DateTime::parse_from_str("2026-07-29 13:00:00", "%Y-%m-%d %H:%M:%S")?.and_utc();

        let wire = to_wire_row(&row, synced);

        assert_eq!(wire.id, 9);
        assert_eq!(wire.created_at.timestamp_subsec_micros(), 123_456);
        assert_eq!(wire.synced_at, synced);
        // A NULL reason stays NULL — the copy is verbatim, not normalized.
        assert_eq!(wire.reason, None);
        Ok(())
    }
}
