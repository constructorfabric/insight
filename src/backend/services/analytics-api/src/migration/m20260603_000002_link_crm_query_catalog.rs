//! Append CRM bullet links to `metric_query_catalog` — the junction
//! table created by `m20260529_000001_metric_query_catalog_link`.
//!
//! Companion to `m20260603_000001_seed_crm_metric_catalog`. Once the
//! eight CRM catalog rows exist, this migration registers which
//! `metrics` row emits which subset, so the FE Layer-2 selector can
//! answer "given a `query_id`, list its catalog ids."
//!
//! ## Why a separate per-query list (not the engineering's `LIKE prefix.%`)
//!
//! The engineering link migration (`m20260529`) uses
//! `WHERE c.metric_key LIKE CONCAT(?, '.%')` because each `metrics`
//! query emits *every* `metric_key` sharing its CH storage-table prefix
//! (e.g. the IC bullet query under `task_delivery_bullet_rows` returns
//! all 11 keys in that table).
//!
//! CRM violates that assumption: both `CRM Bullet Velocity Quality`
//! (`…0022`) and `CRM Bullet Activity` (`…0023`) read FROM
//! `insight.crm_bullet_rows` but each emits a disjoint 4-key subset
//! (Quality: `win_rate`, `avg_deal_size`, `cycle_days`, `deals_opened`;
//! Activity: `calls`, `emails`, `meetings`, `comms_per_won`). A
//! prefix-wide link would over-attach catalog rows to both queries.
//! We list the keys explicitly so the link map matches what each
//! query actually emits, byte-for-byte against
//! `m20260507_000001_seed_crm_metrics::CRM_BULLET_QUALITY_QUERY` /
//! `_ACTIVITY_QUERY`.
//!
//! ## What this migration does NOT do
//!
//! - Does NOT create the junction table — that ships in
//!   `m20260529_000001_metric_query_catalog_link`.
//! - Does NOT alter the engineering link rows — they remain attached
//!   under the original `LIKE prefix.%` rule.
//! - Does NOT touch `crm_kpis` / `crm_chart_flow` / `crm_pipeline_now`.
//!   Those queries emit dashboard-shape rows (not catalog-shape
//!   per-metric_key rows); no catalog coverage exists for them and a
//!   later EPIC can revisit.

use sea_orm::{ConnectionTrait, Statement, Value};
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

/// `(metrics_hex_id, [bare metric_keys])`. Each bare key is concatenated
/// with `crm_bullet_rows.` at INSERT time to build the catalog lookup.
/// Kept explicit (not a prefix wildcard) so a future addition to
/// `crm_bullet_rows` doesn't silently re-attach to both queries — see
/// module-level doc for the rationale.
const CRM_QUERY_LINKS: &[(&str, &[&str])] = &[
    // CRM Bullet Velocity Quality (UUID …0022)
    (
        "00000000000000000001000000000022",
        &["win_rate", "avg_deal_size", "cycle_days", "deals_opened"],
    ),
    // CRM Bullet Activity (UUID …0023)
    (
        "00000000000000000001000000000023",
        &["calls", "emails", "meetings", "comms_per_won"],
    ),
];

/// Wire prefix for every CRM bullet `metric_key` — mirrors the FROM
/// clause of the CRM bullet `query_ref`s.
const CRM_TABLE_PREFIX: &str = "crm_bullet_rows";

/// One `INSERT IGNORE` per `(query, bare_key)` pair. Matching on the
/// exact `<prefix>.<bare_key>` shape (no `LIKE`) so a future row in
/// the same prefix doesn't accidentally get linked.
const INSERT_LINK_SQL: &str = "\
    INSERT IGNORE INTO metric_query_catalog \
        (id, metrics_id, metric_catalog_id) \
    SELECT UNHEX(REPLACE(UUID(),'-','')), UNHEX(?), c.id \
    FROM metric_catalog c \
    WHERE c.metric_key = ?";

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();
        let backend = manager.get_database_backend();

        let mut inserted_pairs = 0usize;
        for (metrics_hex, bare_keys) in CRM_QUERY_LINKS {
            for bare_key in *bare_keys {
                let full_key = format!("{CRM_TABLE_PREFIX}.{bare_key}");
                conn.execute(Statement::from_sql_and_values(
                    backend,
                    INSERT_LINK_SQL,
                    [Value::from(*metrics_hex), Value::from(full_key)],
                ))
                .await?;
                inserted_pairs += 1;
            }
        }

        tracing::info!(
            pairs = inserted_pairs,
            "crm metric_query_catalog backfill applied"
        );

        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        // we have only forward migrations
        Err(DbErr::Custom("we have only forward migrations".to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// Two CRM bullet queries, four bare keys each = 8 expected links.
    /// Bumping this needs an FE-side audit of the CRM bullet emission
    /// shape — silent drift means the link map disagrees with what the
    /// query returns at runtime.
    #[test]
    fn link_count_is_eight() {
        let total: usize = CRM_QUERY_LINKS.iter().map(|(_, keys)| keys.len()).sum();
        assert_eq!(
            total, 8,
            "expected 8 (query, bare_key) pairs (2 queries × 4 keys each)"
        );
    }

    /// Each query maps to a disjoint key subset — Quality and Activity
    /// MUST NOT share keys (the FE bullet rendering relies on the
    /// section split). Catches a copy-paste mistake.
    #[test]
    fn quality_and_activity_keys_are_disjoint() {
        let mut all_keys: HashSet<&str> = HashSet::new();
        for (_metrics_hex, keys) in CRM_QUERY_LINKS {
            for key in *keys {
                assert!(
                    all_keys.insert(*key),
                    "bare key {key:?} appears in more than one CRM query's link list"
                );
            }
        }
    }

    /// Every bare key on the link side MUST exist in the companion seed
    /// migration's SEEDS list — otherwise the INSERT IGNORE silently
    /// no-ops at runtime (no matching `metric_catalog.metric_key`
    /// produces zero rows; the link is invisible).
    #[test]
    fn every_link_key_is_in_seed_migration() {
        // Hard-coded mirror of the seed SEEDS list — keeping this here
        // (instead of importing the const) intentionally so a future
        // edit to the seed forces a deliberate edit here too. If the
        // two get out of sync, the seed seed_count_is_eight pins one
        // side and this list pins the other.
        const SEEDED_BARE_KEYS: &[&str] = &[
            "win_rate",
            "avg_deal_size",
            "cycle_days",
            "deals_opened",
            "calls",
            "emails",
            "meetings",
            "comms_per_won",
        ];
        let seeded: HashSet<&str> = SEEDED_BARE_KEYS.iter().copied().collect();
        for (_metrics_hex, keys) in CRM_QUERY_LINKS {
            for key in *keys {
                assert!(
                    seeded.contains(*key),
                    "link key {key:?} has no matching catalog row in \
                     m20260603_000001 — INSERT IGNORE will silently produce \
                     zero junction rows"
                );
            }
        }
    }

    /// `metrics_hex_id` must be a 32-char lowercase hex string — `UNHEX`
    /// in the INSERT SQL converts it to BINARY(16). Anything else
    /// silently fails the UNHEX (NULL → INSERT skips, no error).
    #[test]
    fn metrics_hex_ids_are_well_formed() {
        for (metrics_hex, _) in CRM_QUERY_LINKS {
            assert_eq!(
                metrics_hex.len(),
                32,
                "metrics_hex {metrics_hex:?} must be 32 chars; \
                 UNHEX fails silently otherwise"
            );
            assert!(
                metrics_hex.chars().all(|c| c.is_ascii_hexdigit()),
                "metrics_hex {metrics_hex:?} must be hex-only; \
                 UNHEX fails silently otherwise"
            );
            assert!(
                metrics_hex.chars().all(|c| !c.is_ascii_uppercase()),
                "metrics_hex {metrics_hex:?} must be lowercase to match the \
                 m20260507_000001_seed_crm_metrics IDs byte-for-byte"
            );
        }
    }

    /// Wire prefix is fixed and matches the seed migration. A typo here
    /// would silently link zero catalog rows.
    #[test]
    fn crm_table_prefix_is_canonical() {
        assert_eq!(
            CRM_TABLE_PREFIX, "crm_bullet_rows",
            "wire prefix MUST match the seed migration's metric_key prefix"
        );
    }
}
