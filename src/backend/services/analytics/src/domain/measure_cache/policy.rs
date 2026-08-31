//! The policy and coverage store. Policy says whether and how often a measure
//! is materialized; coverage says which dates the cache can answer from.

use std::collections::BTreeMap;

use chrono::NaiveDate;
use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseConnection, DbErr, EntityTrait, QueryFilter, Statement,
    Value,
};
use uuid::Uuid;

use crate::domain::compiler::cache_build::CacheRowKind;
use crate::infra::db::entities::{semantic_cache_policies, semantic_measures};

/// How one measure is materialized.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CachePolicy {
    pub measure_key: String,
    pub hot_window_days: u32,
    pub refresh_interval_minutes: u32,
}

pub async fn enabled_policies(db: &DatabaseConnection) -> Result<Vec<CachePolicy>, DbErr> {
    let rows = semantic_cache_policies::Entity::find()
        .filter(semantic_cache_policies::Column::Enabled.eq(true))
        .all(db)
        .await?;

    Ok(rows
        .into_iter()
        .map(|row| CachePolicy {
            measure_key: row.measure_key,
            hot_window_days: row.hot_window_days.unsigned_abs(),
            refresh_interval_minutes: row.refresh_interval_minutes.unsigned_abs(),
        })
        .collect())
}

/// The version each product measure currently stands at. A build always writes
/// at the version the store holds, never at the one a previous run wrote.
pub async fn current_versions(db: &DatabaseConnection) -> Result<BTreeMap<String, u32>, DbErr> {
    let rows = semantic_measures::Entity::find()
        .filter(semantic_measures::Column::TenantId.is_null())
        .filter(semantic_measures::Column::IsEnabled.eq(true))
        .all(db)
        .await?;

    Ok(rows
        .into_iter()
        .map(|row| (row.measure_key, row.definition_version.unsigned_abs()))
        .collect())
}

/// What a landed build leaves the coverage row claiming.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoverageWrite {
    /// The window widens to cover this run plus what earlier runs left behind.
    Widen,
    /// Only this run's window is claimed: what earlier runs left was a
    /// different row shape and has been dropped.
    Replace,
}

/// The row shape the coverage of one measure at one version was built at, or
/// `None` when nothing has been built for that pair.
pub async fn coverage_row_kind(
    db: &DatabaseConnection,
    measure_key: &str,
    definition_version: u32,
) -> Result<Option<CacheRowKind>, DbErr> {
    let rows = db
        .query_all_raw(Statement::from_sql_and_values(
            db.get_database_backend(),
            "SELECT row_kind AS row_kind FROM semantic_cache_coverage \
             WHERE measure_key = ? AND definition_version = ?",
            [
                Value::from(measure_key),
                Value::from(i64::from(definition_version)),
            ],
        ))
        .await?;

    rows.first()
        .map(|row| row.try_get::<String>("", "row_kind"))
        .transpose()
        .map(|kind| kind.as_deref().and_then(CacheRowKind::from_db))
}

/// Written by the refresher alone, after a build lands, and always carrying the
/// row shape that build wrote.
pub async fn record_coverage(
    db: &DatabaseConnection,
    measure_key: &str,
    definition_version: u32,
    kind: CacheRowKind,
    from: NaiveDate,
    to: NaiveDate,
    write: CoverageWrite,
) -> Result<(), DbErr> {
    db.execute_raw(Statement::from_sql_and_values(
        db.get_database_backend(),
        coverage_upsert_sql(write),
        [
            Value::Bytes(Some(Uuid::now_v7().as_bytes().to_vec())),
            Value::from(measure_key),
            Value::from(i64::from(definition_version)),
            Value::from(kind.as_db()),
            Value::from(from.to_string()),
            Value::from(to.to_string()),
        ],
    ))
    .await?;
    Ok(())
}

// INVARIANT: `row_kind` is rewritten by both forms, so a coverage row never
// states a shape other than the one the last landed build wrote.
fn coverage_upsert_sql(write: CoverageWrite) -> &'static str {
    match write {
        CoverageWrite::Widen => {
            "INSERT INTO semantic_cache_coverage \
                (id, measure_key, definition_version, row_kind, covered_from, covered_to) \
             VALUES (?, ?, ?, ?, ?, ?) \
             ON DUPLICATE KEY UPDATE \
                row_kind = VALUES(row_kind), \
                covered_from = LEAST(covered_from, VALUES(covered_from)), \
                covered_to = GREATEST(covered_to, VALUES(covered_to)), \
                refreshed_at = CURRENT_TIMESTAMP(3)"
        }
        CoverageWrite::Replace => {
            "INSERT INTO semantic_cache_coverage \
                (id, measure_key, definition_version, row_kind, covered_from, covered_to) \
             VALUES (?, ?, ?, ?, ?, ?) \
             ON DUPLICATE KEY UPDATE \
                row_kind = VALUES(row_kind), \
                covered_from = VALUES(covered_from), \
                covered_to = VALUES(covered_to), \
                refreshed_at = CURRENT_TIMESTAMP(3)"
        }
    }
}

/// INVARIANT: seeding writes the default once and never overwrites it — the
/// enablement and cadence an operator set are theirs, and no column here can
/// move a definition version.
pub async fn seed_cache_policies<C: ConnectionTrait>(
    conn: &C,
    measure_keys: impl IntoIterator<Item = &str>,
) -> Result<(), DbErr> {
    for measure_key in measure_keys {
        conn.execute_raw(Statement::from_sql_and_values(
            conn.get_database_backend(),
            "INSERT INTO semantic_cache_policies (id, measure_key, enabled) VALUES (?, ?, TRUE) \
             ON DUPLICATE KEY UPDATE measure_key = measure_key",
            [
                Value::Bytes(Some(Uuid::now_v7().as_bytes().to_vec())),
                Value::from(measure_key),
            ],
        ))
        .await?;
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn a_widened_window_keeps_what_earlier_runs_left_and_a_replaced_one_does_not() {
        let widen = coverage_upsert_sql(CoverageWrite::Widen);
        let replace = coverage_upsert_sql(CoverageWrite::Replace);

        assert!(
            widen.contains("LEAST(covered_from, VALUES(covered_from))"),
            "{widen}"
        );
        assert!(
            widen.contains("GREATEST(covered_to, VALUES(covered_to))"),
            "{widen}"
        );
        assert!(!replace.contains("LEAST("), "{replace}");
        assert!(!replace.contains("GREATEST("), "{replace}");
        assert!(
            replace.contains("covered_from = VALUES(covered_from)"),
            "{replace}"
        );
        assert!(
            replace.contains("covered_to = VALUES(covered_to)"),
            "{replace}"
        );
    }

    #[test]
    fn every_coverage_write_states_the_row_shape_the_build_wrote() {
        for write in [CoverageWrite::Widen, CoverageWrite::Replace] {
            let sql = coverage_upsert_sql(write);

            assert!(
                sql.contains("row_kind = VALUES(row_kind)"),
                "{write:?}: {sql}"
            );
            assert_eq!(sql.matches('?').count(), 6, "{write:?}: {sql}");
        }
    }
}
