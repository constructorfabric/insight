//! The analytics schema the first 67 migrations built, in one step. Tables that
//! `m20261014_000001_drop_unread_catalog_tables` drops, and the rows seeded
//! into them, are left out.
//!
//! A fresh database gets the schema here; a database that walked the whole
//! squashed history already holds it, and this migration applies nothing. See
//! `insight_migration::squash`.

mod schema;

use insight_migration::squash::{self, History};
use sea_orm_migration::prelude::*;

pub use schema::{
    REQUIRED_DEFINITION_CHECKS, REQUIRED_DIMENSION_CHECKS, REQUIRED_SOURCE_CHECKS,
    REQUIRED_SOURCE_DIMENSION_CHECKS, REQUIRED_SOURCE_MEASURE_CHECKS, REQUIRED_TAG_CHECKS,
};

/// Releases that walk a database through the whole squashed history.
const UPGRADE_THROUGH: &str = "insight umbrella 0.7.169, or a release-2026.09 build from 0.6.92 on";

// INVARIANT: these are the ledger names of the squashed migrations, in their
// original order. Installed databases record them; renaming, dropping or
// reordering one breaks `migrate` and the boot gate on those databases.
pub const SQUASHED: &[&str] = &[
    "m20260414_000001_init",
    "m20260422_000001_seed_metrics",
    "m20260423_000001_seed_metrics_honest_nulls",
    "m20260428_000001_collab_metrics_update",
    "m20260429_000001_task_delivery_silver_rewrite",
    "m20260430_000001_update_git_bullet",
    "m20260507_000001_seed_crm_metrics",
    "m20260515_000001_task_delivery_bullet_rewrite",
    "m20260518_000001_collab_bullet_rewrite",
    "m20260519_000001_ai_bullet_rewrite",
    "m20260520_000001_code_quality_bullet_rewrite",
    "m20260522_000001_metric_catalog",
    "m20260522_000002_metric_threshold",
    "m20260522_000003_threshold_lock_audit",
    "m20260527_000001_seed_metric_catalog",
    "m20260527_000002_seed_metric_views",
    "m20260529_000001_metric_query_catalog_link",
    "m20260601_000001_ai_claude_team_metrics",
    "m20260601_000002_seed_claude_team_metrics_catalog",
    "m20260603_000001_seed_crm_metric_catalog",
    "m20260603_000002_link_crm_query_catalog",
    "m20260604_000001_task_delivery_bullet_distribution",
    "m20260604_000002_collab_bullet_distribution",
    "m20260604_000004_code_quality_bullet_distribution",
    "m20260604_000005_git_bullet_distribution",
    "m20260604_000006_ic_kpis_peer_median",
    "m20260604_000009_member_metric_values",
    "m20260606_000001_dept_metric_distributions",
    "m20260609_000001_ai_chatgpt_team_metrics",
    "m20260609_000002_seed_chatgpt_team_metrics_catalog",
    "m20260610_000001_fix_ai_label_drift",
    "m20260611_000001_ai_bullet_dept_reconcile",
    "m20260612_000001_support_metrics",
    "m20260612_000001_team_git_bullet",
    "m20260612_000002_seed_support_catalog",
    "m20260612_000003_link_support_query_catalog",
    "m20260618_000001_ai_claude_team_overage_metric",
    "m20260618_000002_seed_claude_team_overage_catalog",
    "m20260620_000001_seed_wiki_metrics",
    "m20260620_000002_seed_wiki_catalog",
    "m20260623_000001_ai_personal_queries",
    "m20260623_000002_seed_ai_personal_metric_catalog",
    "m20260624_000001_collab_zulip_chat",
    "m20260624_000002_seed_zulip_collab_catalog",
    "m20260624_000003_ic_chart_loc_git_breakdown",
    "m20260625_000001_metric_definitions",
    "m20260701_000001_devendor_email_labels",
    "m20260702_000001_collab_messaging_queries",
    "m20260702_000002_seed_collab_messaging_catalog",
    "m20260709_000001_metric_median_computation",
    "m20260710_000001_metric_distinct_count_computation",
    "m20260714_000001_metric_value_transform",
    "m20260721_000001_metric_definition_short_label",
    "m20260722_000001_metric_definition_last_observed",
    "m20260727_000001_metric_evidence",
    "m20260730_000001_saved_queries",
    "m20260805_000001_semantic_definition_core",
    "m20260806_000001_metric_custom_observation_sql",
    "m20260810_000001_metric_definition_subject",
    "m20260810_000002_metric_definition_tags",
    "m20260811_000001_metric_source_measure_alias_collapse",
    "m20260822_000001_feedback",
    "m20260822_000002_ai_assist",
    "m20260823_000001_ratio_denominator_aggregation",
    "m20260824_000001_metric_percentile_stddev_computation",
    "m20260825_000001_metric_evidence_presentation",
    "m20260903_000001_metric_definition_first_observed",
];

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        match squash::history::<super::Migrator>(manager, SQUASHED, UPGRADE_THROUGH).await? {
            History::Fresh => create_schema(manager).await,
            History::Complete => Ok(()),
        }
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom("we have only forward migrations".to_owned()))
    }
}

async fn create_schema(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    let conn = manager.get_connection();
    for statement in schema::STATEMENTS {
        conn.execute_unprepared(statement).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_squashed_name_is_listed_once() {
        let mut seen = std::collections::HashSet::new();
        for name in SQUASHED {
            assert!(seen.insert(*name), "squashed twice: {name}");
        }
    }
}
