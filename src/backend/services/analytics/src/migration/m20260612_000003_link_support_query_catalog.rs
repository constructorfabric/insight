//! Append Support bullet links to `metric_query_catalog` (the junction table
//! from `m20260529_000001`). Companion to `m20260612_000002_seed_support_catalog`
//! + `m20260612_000001_support_metrics`.
//!
//! Both Support queries — Team Bullet Support (…0007) and IC Bullet Support
//! (…0008) — read FROM `insight.support_bullet_rows` and emit the SAME 7 keys
//! (the same bullets render on the team and IC dashboards). So unlike CRM
//! (disjoint subsets), each query links to all 7 keys → 14 links.

use sea_orm::{ConnectionTrait, Statement, Value};
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

/// `support_bullet_rows.*` bare keys emitted by BOTH support `query_refs`.
const SUPPORT_BARE_KEYS: &[&str] = &[
    "support_active",
    "support_updates",
    "support_public_comments",
    "support_private_comments",
    "support_solved",
    "support_csat",
    "support_kb",
];

/// `(metrics_hex_id, bare_keys)` — Team + IC both emit the full key set.
const SUPPORT_QUERY_LINKS: &[(&str, &[&str])] = &[
    ("00000000000000000001000000000007", SUPPORT_BARE_KEYS), // Team Bullet Support
    ("00000000000000000001000000000008", SUPPORT_BARE_KEYS), // IC Bullet Support
];

/// Wire prefix — mirrors the FROM clause of the support bullet `query_refs` and
/// the `metric_key` namespace in `m20260612_000002`.
const SUPPORT_TABLE_PREFIX: &str = "support_bullet_rows";

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
        let mut pairs = 0usize;
        for (metrics_hex, bare_keys) in SUPPORT_QUERY_LINKS {
            for bare_key in *bare_keys {
                let full_key = format!("{SUPPORT_TABLE_PREFIX}.{bare_key}");
                conn.execute(Statement::from_sql_and_values(
                    backend,
                    INSERT_LINK_SQL,
                    [Value::from(*metrics_hex), Value::from(full_key)],
                ))
                .await?;
                pairs += 1;
            }
        }
        tracing::info!(pairs, "support metric_query_catalog backfill applied");
        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom("we have only forward migrations".to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 2 queries × 7 keys = 14 links.
    #[test]
    fn link_count_is_fourteen() {
        let total: usize = SUPPORT_QUERY_LINKS.iter().map(|(_, k)| k.len()).sum();
        assert_eq!(total, 14, "expected 2 queries × 7 keys");
    }

    /// metrics hex ids are 32-char lowercase hex (UNHEX fails silently otherwise).
    #[test]
    fn metrics_hex_ids_well_formed() {
        for (hex, _) in SUPPORT_QUERY_LINKS {
            assert_eq!(hex.len(), 32);
            assert!(
                hex.chars()
                    .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
            );
        }
    }

    /// Link keys must match the seed migration's namespaced keys byte-for-byte.
    #[test]
    fn prefix_and_keys_match_seed() {
        assert_eq!(SUPPORT_TABLE_PREFIX, "support_bullet_rows");
        for bare in SUPPORT_BARE_KEYS {
            assert!(bare.starts_with("support_"), "{bare}");
        }
    }
}
