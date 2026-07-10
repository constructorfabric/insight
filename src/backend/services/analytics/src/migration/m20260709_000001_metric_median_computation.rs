//! Extends the metric computation vocabulary with `median`.
//!
//! `median` metrics aggregate per-event observation values with
//! `quantileExact(0.5)`; like `sum` they carry no `scale`. The CHECK
//! constraint is dropped and re-added under the same name so the startup
//! CHECK probe (`REQUIRED_DEFINITION_CHECKS`) stays green, and the enum
//! value is appended at the tail so existing rows keep their ordinals.

use sea_orm_migration::prelude::*;

// IF EXISTS makes this idempotent forward-repair: a warm cluster whose
// constraint was already dropped (incident recovery, or a MariaDB that
// parsed but never persisted it) must still reach the enum widen rather than
// abort the migration and refuse to start.
const DROP_CHECK: &str = "ALTER TABLE metric_definitions \
     DROP CONSTRAINT IF EXISTS chk_metric_definitions_computation_fields";

const EXTEND_ENUM: &str = "ALTER TABLE metric_definitions \
     MODIFY COLUMN computation_type ENUM('sum','ratio','median') NOT NULL";

const READD_CHECK: &str = "ALTER TABLE metric_definitions \
     ADD CONSTRAINT chk_metric_definitions_computation_fields CHECK ( \
        (computation_type = 'sum' AND scale IS NULL) \
        OR (computation_type = 'ratio' AND scale IS NOT NULL) \
        OR (computation_type = 'median' AND scale IS NULL) \
     )";

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        db.execute_unprepared(DROP_CHECK).await?;
        db.execute_unprepared(EXTEND_ENUM).await?;
        db.execute_unprepared(READD_CHECK).await?;
        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Migration(
            "m20260709_000001_metric_median_computation is irreversible: \
             median definitions may exist; narrowing the enum would corrupt them."
                .to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The reconciler writes `median` only after this migration ran, so the
    /// enum text and the three-way CHECK rule must stay in lockstep with
    /// `MetricComputation::as_db` and the constraint name the startup probe
    /// asserts.
    #[test]
    fn alter_statements_pin_enum_and_check() {
        assert!(EXTEND_ENUM.contains("ENUM('sum','ratio','median')"));
        for statement in [DROP_CHECK, READD_CHECK] {
            assert!(statement.contains("chk_metric_definitions_computation_fields"));
        }
        assert!(READD_CHECK.contains("(computation_type = 'sum' AND scale IS NULL)"));
        assert!(READD_CHECK.contains("(computation_type = 'ratio' AND scale IS NOT NULL)"));
        assert!(READD_CHECK.contains("(computation_type = 'median' AND scale IS NULL)"));
    }
}
