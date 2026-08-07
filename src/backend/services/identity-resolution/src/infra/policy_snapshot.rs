//! ClickHouse publisher for `identity.person_attribute_policy_snapshot` —
//! the analytical projection of the person-attribute registry's current
//! policy, so the query path enforces policy without calling this service.
//!
//! Keys are the RAW warehouse strings the registry stores, so the snapshot
//! joins the claim relations byte-equal (see `sql/015_person_attributes.sql`).
//! Publishing mechanics live in [`crate::infra::snapshot_writer`].

use std::time::Duration;

use chrono::Utc;
use clickhouse::Row;
use serde::Serialize;
use uuid::Uuid;

use crate::infra::db::person_attributes_repo::CurrentPolicyRow;
use crate::infra::snapshot_writer::{SnapshotSpec, SnapshotWriter};

const WRITE_TIMEOUT: Duration = Duration::from_mins(5);

/// One row per definition carrying its current (highest) policy revision.
/// Ordered by the join key the query path filters on.
const COLUMNS_DDL: &str = r"
    definition_id       UUID,
    insight_tenant_id   String,
    insight_source_type String,
    insight_source_id   String,
    source_field_id     String,
    revision            Int32,
    label_override      Nullable(String),
    sensitivity_class   Nullable(String),
    grouping_enabled    Bool,
    comparison_enabled  Bool,
    value_mode          LowCardinality(String),
    retired             Bool,
    _published_at       DateTime64(3, 'UTC')
";

pub(crate) const SPEC: SnapshotSpec = SnapshotSpec {
    database: "identity",
    target: "person_attribute_policy_snapshot",
    staging_prefix: "person_attribute_policy_snapshot_staging_",
    columns_ddl: COLUMNS_DDL,
    order_by: "(insight_tenant_id, insight_source_type, insight_source_id, source_field_id)",
    watermark_column: "_published_at",
    log_label: "policy-publish",
};

#[derive(Debug, Row, Serialize)]
struct WireRow {
    #[serde(with = "clickhouse::serde::uuid")]
    definition_id: Uuid,
    insight_tenant_id: String,
    insight_source_type: String,
    insight_source_id: String,
    source_field_id: String,
    revision: i32,
    label_override: Option<String>,
    sensitivity_class: Option<String>,
    grouping_enabled: bool,
    comparison_enabled: bool,
    value_mode: String,
    retired: bool,
    #[serde(
        rename = "_published_at",
        with = "clickhouse::serde::chrono::datetime64::millis"
    )]
    published_at: chrono::DateTime<Utc>,
}

/// Publishes the current-policy projection.
pub struct ClickHousePolicySnapshotWriter {
    writer: SnapshotWriter,
}

impl ClickHousePolicySnapshotWriter {
    /// Build a writer from connection settings (empty user → no auth).
    #[must_use]
    pub fn connect(url: &str, user: &str, password: &str) -> Self {
        Self {
            writer: SnapshotWriter::connect(url, user, password, SPEC, WRITE_TIMEOUT),
        }
    }

    /// Replace the published snapshot with `rows`.
    ///
    /// # Errors
    ///
    /// Returns an error if publishing fails; the live relation is untouched
    /// unless the atomic swap succeeded.
    pub async fn replace(
        &self,
        rows: &[CurrentPolicyRow],
        published_at: chrono::DateTime<Utc>,
    ) -> anyhow::Result<()> {
        let wire: Vec<WireRow> = rows.iter().map(|r| to_wire_row(r, published_at)).collect();
        self.writer.replace(&wire, published_at).await
    }

    /// Rows in the live relation — half of the caller's published-state check.
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails (including a missing relation,
    /// which must not be read as "nothing published").
    pub async fn published_row_count(&self) -> anyhow::Result<u64> {
        self.writer.published_row_count().await
    }
}

fn to_wire_row(r: &CurrentPolicyRow, published_at: chrono::DateTime<Utc>) -> WireRow {
    WireRow {
        definition_id: r.definition_id,
        insight_tenant_id: r.insight_tenant_id.clone(),
        insight_source_type: r.insight_source_type.clone(),
        insight_source_id: r.insight_source_id.clone(),
        source_field_id: r.source_field_id.clone(),
        revision: r.revision,
        label_override: r.label_override.clone(),
        sensitivity_class: r.sensitivity_class.clone(),
        grouping_enabled: r.grouping_enabled,
        comparison_enabled: r.comparison_enabled,
        value_mode: r.value_mode.as_db().to_owned(),
        retired: r.retired,
        published_at,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::db::person_attributes_repo::ValueMode;

    fn row() -> CurrentPolicyRow {
        CurrentPolicyRow {
            definition_id: Uuid::from_u128(7),
            insight_tenant_id: "Tenant-Raw".to_owned(),
            insight_source_type: "bamboohr".to_owned(),
            insight_source_id: "hr-main".to_owned(),
            source_field_id: "jobTitle".to_owned(),
            revision: 3,
            label_override: None,
            sensitivity_class: Some("restricted".to_owned()),
            grouping_enabled: true,
            comparison_enabled: false,
            value_mode: ValueMode::Single,
            retired: false,
        }
    }

    #[test]
    fn publishes_registry_keys_verbatim() -> anyhow::Result<()> {
        let at = chrono::DateTime::parse_from_rfc3339("2026-08-06T10:00:00Z")?.with_timezone(&Utc);

        let wire = to_wire_row(&row(), at);

        // Byte-equal keys are the contract: the query path joins these against
        // the claim relations without normalizing either side.
        assert_eq!(wire.insight_tenant_id, "Tenant-Raw");
        assert_eq!(wire.insight_source_id, "hr-main");
        assert_eq!(wire.value_mode, "single");
        assert_eq!(wire.published_at, at);
        Ok(())
    }
}
