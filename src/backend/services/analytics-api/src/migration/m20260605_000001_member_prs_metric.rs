//! `Member PRs Merged` metric for the V2 team heatmap's PRs column.
//!
//! The team heatmap needs per-person PRs-merged for the roster, period-bounded.
//! The data lives only in `silver.mtr_git_person_weekly` (per person, per week):
//! `git_bullet_rows.prs_merged` is empty, and `team_member.prs_merged` is a
//! NULL placeholder. We read the canonical weekly silver directly (NOT
//! `insight.ic_kpis`) so the team view stays decoupled from the IC dashboard —
//! both surfaces independently derive the same number from the same source.
//!
//! Shape: per-person long rows scoped by `person_id IN (roster)`. The inner
//! subquery normalizes the silver's `person_key`/`week` to `person_id`/
//! `metric_date` so the handler's date-range filter binds; the outer sums each
//! person's weekly rows over the selected period (no fanout — each week once).

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const ZERO_TENANT: &str = "00000000000000000000000000000000";
const MEMBER_PRS_HEX: &str = "00000000000000000001000000000043";

const QUERY_REF: &str = "SELECT person_id, sum(prs_merged) AS prs_merged FROM (SELECT person_key AS person_id, week AS metric_date, prs_merged FROM silver.mtr_git_person_weekly) GROUP BY person_id";

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        db.execute_unprepared(&format!(
            "INSERT INTO metrics (id, insight_tenant_id, name, description, query_ref, is_enabled) \
             VALUES (UNHEX('{MEMBER_PRS_HEX}'), UNHEX('{ZERO_TENANT}'), 'Member PRs Merged', \
             'Per-person PRs merged for a roster (person_id IN), period-bounded, from silver.mtr_git_person_weekly.', \
             '{qr}', 1) \
             ON DUPLICATE KEY UPDATE name=VALUES(name), description=VALUES(description), query_ref=VALUES(query_ref), is_enabled=1",
            qr = QUERY_REF.replace('\'', "''"),
        ))
        .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        db.execute_unprepared(&format!(
            "DELETE FROM metrics WHERE id = UNHEX('{MEMBER_PRS_HEX}')"
        ))
        .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_shape() {
        assert!(QUERY_REF.contains("FROM silver.mtr_git_person_weekly"));
        assert!(QUERY_REF.contains("week AS metric_date"), "must normalize week for date-filter injection");
        assert!(QUERY_REF.contains("sum(prs_merged)"));
        assert!(QUERY_REF.contains("GROUP BY person_id"));
        // Decoupled: must NOT read the IC dashboard view.
        assert!(!QUERY_REF.contains("ic_kpis"));
    }
}
