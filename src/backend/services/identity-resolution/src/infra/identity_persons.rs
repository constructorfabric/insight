use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use clickhouse::Row;
use insight_clickhouse::{Client, Config, Ddl, TableSpec, Topology};
use sea_orm::prelude::DateTime;
use serde::Serialize;
use uuid::Uuid;

use crate::domain::sync_service::{IdentityPersonsWriter, PersonsLogRow};

const WRITE_TIMEOUT: Duration = Duration::from_mins(5);

const DATABASE: &str = "identity";
const TARGET: &str = "identity_persons";
const STAGING_PREFIX: &str = "identity_persons_staging_";
const STAGING_GC_AGE_SECONDS: u32 = 3600;

const COLUMNS: [(&str, &str); 14] = [
    ("id", "UInt64"),
    ("value_type", "String"),
    ("insight_source_type", "String"),
    ("insight_source_id", "UUID"),
    ("insight_tenant_id", "UUID"),
    ("value_id", "Nullable(String)"),
    ("value_full_text", "Nullable(String)"),
    ("value", "Nullable(String)"),
    ("value_effective", "Nullable(String)"),
    ("person_id", "UUID"),
    ("author_person_id", "UUID"),
    ("reason", "Nullable(String)"),
    ("created_at", "DateTime64(6, 'UTC')"),
    ("_synced_at", "DateTime64(3, 'UTC')"),
];
const SORT_KEY: [&str; 1] = ["id"];

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Target {
    Published,
    Absent,
}

fn staging_spec(staging: &str) -> Ddl {
    Ddl::create_table(TableSpec::merge_tree(
        DATABASE, staging, &COLUMNS, &SORT_KEY,
    ))
}

fn swap_ddl(staging: &str, target: Target) -> Ddl {
    match target {
        Target::Published => Ddl::exchange_tables(DATABASE, staging, TARGET),
        Target::Absent => Ddl::rename_table(DATABASE, staging, TARGET),
    }
}

pub struct ClickHouseIdentityPersonsWriter {
    client: Client,
}

impl ClickHouseIdentityPersonsWriter {
    #[must_use]
    pub fn new(client: Client) -> Self {
        Self { client }
    }

    #[must_use]
    pub fn connect(url: &str, user: &str, password: &str, topology: Topology) -> Self {
        let mut config = Config::new(url, DATABASE)
            .with_query_timeout(WRITE_TIMEOUT)
            .with_topology(topology);
        if !user.is_empty() {
            config = config.with_auth(user, password);
        }
        Self::new(Client::new(config))
    }

    async fn target_state(&self) -> anyhow::Result<Target> {
        let created: u64 = self
            .client
            .query("SELECT count() FROM system.tables WHERE database = ? AND name = ?")
            .bind(DATABASE)
            .bind(TARGET)
            .fetch_one()
            .await?;
        Ok(if created == 0 {
            Target::Absent
        } else {
            Target::Published
        })
    }

    async fn guard_watermark(&self, synced_at: chrono::DateTime<Utc>) -> anyhow::Result<()> {
        let published_ms: i64 = self
            .client
            .consistent_query(&format!(
                "SELECT toUnixTimestamp64Milli(max(_synced_at)) FROM {DATABASE}.{TARGET}"
            ))
            .fetch_one()
            .await?;
        anyhow::ensure!(
            published_ms <= synced_at.timestamp_millis(),
            "a newer snapshot (_synced_at={published_ms}ms) is already published; \
             discarding this run's older snapshot ({}ms)",
            synced_at.timestamp_millis()
        );
        Ok(())
    }

    async fn drop_stale_stagings(&self) {
        let stale: Result<Vec<String>, _> = self
            .client
            .query(
                "SELECT name FROM system.tables \
                 WHERE database = ? AND name LIKE ? \
                   AND metadata_modification_time < now() - INTERVAL ? SECOND",
            )
            .bind(DATABASE)
            .bind(format!("{STAGING_PREFIX}%"))
            .bind(STAGING_GC_AGE_SECONDS)
            .fetch_all()
            .await;
        let stale = match stale {
            Ok(names) => names,
            Err(e) => {
                tracing::warn!(error = %e, "persons-sync: staging GC listing failed (skipped)");
                return;
            }
        };
        for name in stale {
            let Some(suffix) = name.strip_prefix(STAGING_PREFIX) else {
                continue;
            };
            if suffix.len() != 32 || !suffix.bytes().all(|b| b.is_ascii_hexdigit()) {
                continue;
            }
            match self
                .client
                .execute_ddl(&Ddl::drop_table(DATABASE, &name))
                .await
            {
                Ok(()) => tracing::info!(table = %name, "persons-sync: dropped orphaned staging"),
                Err(e) => {
                    tracing::warn!(error = %e, table = %name, "persons-sync: staging GC drop failed");
                }
            }
        }
    }

    async fn fill_and_swap(
        &self,
        staging: &str,
        rows: &[PersonsLogRow],
        synced_at: chrono::DateTime<Utc>,
    ) -> anyhow::Result<()> {
        let mut insert = self.client.consistent_insert::<WireRow>(staging).await?;
        for row in rows {
            insert.write(&to_wire_row(row, synced_at)).await?;
        }
        insert.end().await?;

        let count: u64 = self
            .client
            .consistent_query(&format!("SELECT count() FROM {DATABASE}.`{staging}`"))
            .fetch_one()
            .await?;
        let expected = rows.len() as u64;
        anyhow::ensure!(
            count == expected,
            "staging count mismatch: inserted {expected}, staging holds {count}; \
             aborting swap (live table left untouched)"
        );

        let target = self.target_state().await?;
        match target {
            Target::Published => self.guard_watermark(synced_at).await?,
            Target::Absent => {}
        }

        self.client.execute_ddl(&swap_ddl(staging, target)).await?;
        Ok(())
    }

    async fn probe_published_max_id(&self) -> anyhow::Result<Option<u64>> {
        let probed: Result<Vec<Option<u64>>, _> = self
            .client
            .query(&format!(
                "SELECT if(count() = 0, NULL, max(id)) FROM {DATABASE}.{TARGET}"
            ))
            .fetch_all()
            .await;
        match probed {
            Ok(rows) => Ok(rows.into_iter().next().flatten()),
            Err(e) => {
                tracing::warn!(error = %e, "identity_persons probe failed; publishing");
                Ok(None)
            }
        }
    }
}

#[async_trait]
impl IdentityPersonsWriter for ClickHouseIdentityPersonsWriter {
    async fn published_max_id(&self) -> anyhow::Result<Option<u64>> {
        self.probe_published_max_id().await
    }

    async fn replace(&self, rows: &[PersonsLogRow], synced_at: DateTime) -> anyhow::Result<()> {
        let synced_at = synced_at.and_utc();
        let staging = format!("{STAGING_PREFIX}{}", Uuid::now_v7().simple());

        self.client.assert_database_topology(DATABASE).await?;
        self.drop_stale_stagings().await;

        self.client.execute_ddl(&staging_spec(&staging)).await?;

        let result = self.fill_and_swap(&staging, rows, synced_at).await;

        if let Err(e) = self
            .client
            .execute_ddl(&Ddl::drop_table(DATABASE, &staging))
            .await
        {
            tracing::warn!(error = %e, table = %staging, "persons-sync: dropping own staging failed");
        }
        result
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
        assert_eq!(wire.reason, None);
        Ok(())
    }

    fn swap(target: Target) -> anyhow::Result<String> {
        Ok(writer(Topology::Standalone)
            .client
            .ddl_sql(&swap_ddl("identity_persons_staging_abc", target))?)
    }

    #[test]
    fn a_published_table_is_swapped_for_atomically() -> anyhow::Result<()> {
        assert_eq!(
            swap(Target::Published)?,
            "EXCHANGE TABLES `identity`.`identity_persons_staging_abc` \
             AND `identity`.`identity_persons`"
        );
        Ok(())
    }

    #[test]
    fn a_table_no_creator_has_made_yet_is_renamed_into_place() -> anyhow::Result<()> {
        assert_eq!(
            swap(Target::Absent)?,
            "RENAME TABLE `identity`.`identity_persons_staging_abc` \
             TO `identity`.`identity_persons`"
        );
        Ok(())
    }

    fn writer(topology: Topology) -> ClickHouseIdentityPersonsWriter {
        ClickHouseIdentityPersonsWriter::connect("http://ch:8123", "", "", topology)
    }

    #[test]
    fn a_clustered_install_stages_into_a_replicated_table() -> anyhow::Result<()> {
        let clustered = writer(Topology::new(true, "insight_cluster"))
            .client
            .ddl_sql(&staging_spec("identity_persons_staging_abc"))?;
        let standalone = writer(Topology::Standalone)
            .client
            .ddl_sql(&staging_spec("identity_persons_staging_abc"))?;

        assert!(
            clustered.ends_with(") ENGINE = ReplicatedMergeTree ORDER BY id"),
            "{clustered}"
        );
        assert!(
            standalone.ends_with(") ENGINE = MergeTree ORDER BY id"),
            "{standalone}"
        );
        assert!(!clustered.contains("ON CLUSTER"), "{clustered}");
        Ok(())
    }

    #[test]
    fn the_staging_table_is_the_shape_the_swap_publishes() -> anyhow::Result<()> {
        let sql = writer(Topology::Standalone)
            .client
            .ddl_sql(&staging_spec("identity_persons_staging_abc"))?;

        assert!(
            sql.starts_with("CREATE TABLE `identity`.`identity_persons_staging_abc` ("),
            "{sql}"
        );
        assert!(sql.ends_with(") ENGINE = MergeTree ORDER BY id"), "{sql}");
        for (name, data_type) in COLUMNS {
            assert!(
                sql.contains(&format!("\n    {name} {data_type}")),
                "`{name} {data_type}` must be declared: {sql}"
            );
        }
        Ok(())
    }

    #[test]
    fn publishing_never_creates_the_table_dbt_owns() -> anyhow::Result<()> {
        for target in [Target::Published, Target::Absent] {
            let sql = swap(target)?;

            assert!(
                !sql.contains("CREATE"),
                "the swap must not become a second creator: {sql}"
            );
        }
        Ok(())
    }
}
