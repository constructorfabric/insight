//! Startup schema validation. Fails fast if the expected columns are missing on
//! `staging.jira__task_field_history` — this protects against running against a
//! stale dbt-managed DDL (see ADR-003). The Rust binary writes its output into
//! this staging table, which dbt then unions into `silver.class_task_field_history`.

use super::IoError;
use super::ch_client::ChConfig;
use clickhouse::Row;
use serde::Deserialize;

const REQUIRED_FIELD_HISTORY_COLUMNS: &[&str] = &[
    "unique_key",
    "insight_source_id",
    "data_source",
    "issue_id",
    "id_readable",
    "event_id",
    "event_at",
    "event_kind",
    "author_id",
    "author_display",
    "field_id",
    "field_name",
    "field_cardinality",
    "delta_action",
    "delta_value_id",
    "delta_value_display",
    "value_ids",
    "value_displays",
    "value_id_type",
    "collected_at",
    "_version",
];

#[derive(Row, Deserialize, Debug)]
struct ColumnRow {
    name: String,
    // `r#type` reserved; leave out for now — we validate presence, not types.
}

/// Assert that `staging.jira__task_field_history` has all required columns.
/// Returns an error with the set of missing column names.
pub async fn validate_field_history(cfg: &ChConfig) -> Result<(), IoError> {
    let client = cfg.client();

    let rows: Vec<ColumnRow> = client
        .query(
            "SELECT name FROM system.columns \
             WHERE database = 'staging' AND table = 'jira__task_field_history'",
        )
        .fetch_all()
        .await?;

    let present: std::collections::HashSet<&str> = rows.iter().map(|r| r.name.as_str()).collect();

    let missing: Vec<&&str> = REQUIRED_FIELD_HISTORY_COLUMNS
        .iter()
        .filter(|c| !present.contains(*c as &str))
        .collect();

    if !missing.is_empty() {
        return Err(IoError::SchemaMismatch(format!(
            "staging.jira__task_field_history missing columns: {missing:?}"
        )));
    }

    Ok(())
}
