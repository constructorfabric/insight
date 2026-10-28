use sea_orm_migration::prelude::*;

// INVARIANT: `metric_query_catalog` references `metrics` and `metric_catalog`, so
// it drops first; `IF EXISTS` lets a run interrupted between DROPs finish on retry.
const DROPS: &[&str] = &[
    "DROP TABLE IF EXISTS metric_query_catalog",
    "DROP TABLE IF EXISTS metrics",
    "DROP TABLE IF EXISTS metric_catalog",
    "DROP TABLE IF EXISTS metric_threshold",
    "DROP TABLE IF EXISTS threshold_lock_audit",
    "DROP TABLE IF EXISTS thresholds",
    "DROP TABLE IF EXISTS table_columns",
    "DROP TABLE IF EXISTS semantic_definition_revisions",
    "DROP TABLE IF EXISTS semantic_metrics",
    "DROP TABLE IF EXISTS semantic_measures",
    "DROP TABLE IF EXISTS semantic_datasets",
];

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();
        for statement in DROPS {
            conn.execute_unprepared(statement).await?;
        }
        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom("we have only forward migrations".to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::DROPS;

    fn position(table: &str) -> usize {
        let statement = format!("DROP TABLE IF EXISTS {table}");
        let Some(index) = DROPS.iter().position(|s| *s == statement) else {
            panic!("{table} is not dropped");
        };
        index
    }

    #[test]
    fn a_referencing_table_drops_before_the_tables_it_references() {
        let child = position("metric_query_catalog");
        for parent in ["metrics", "metric_catalog"] {
            assert!(
                child < position(parent),
                "metric_query_catalog must drop before {parent}"
            );
        }
    }
}
