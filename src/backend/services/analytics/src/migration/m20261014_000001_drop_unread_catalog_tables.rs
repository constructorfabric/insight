use sea_orm_migration::prelude::*;

// INVARIANT: only tables no released image reads or probes at boot; a rolled-back
// image must still start. `metric_query_catalog` references `metrics`, so it drops first.
const DROPS: &[&str] = &[
    "DROP TABLE IF EXISTS metric_query_catalog",
    "DROP TABLE IF EXISTS metrics",
    "DROP TABLE IF EXISTS thresholds",
    "DROP TABLE IF EXISTS table_columns",
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
    fn a_referencing_table_drops_before_the_table_it_references() {
        assert!(
            position("metric_query_catalog") < position("metrics"),
            "metric_query_catalog must drop before metrics"
        );
    }

    #[test]
    fn tables_a_previous_image_probes_at_boot_are_kept() {
        for table in [
            "metric_catalog",
            "metric_threshold",
            "threshold_lock_audit",
            "semantic_datasets",
            "semantic_measures",
            "semantic_metrics",
            "semantic_definition_revisions",
        ] {
            let statement = format!("DROP TABLE IF EXISTS {table}");
            assert!(
                !DROPS.contains(&statement.as_str()),
                "{table} must outlive the release that stopped probing it"
            );
        }
    }
}
