//! Database migrations for the Analytics API service.

mod m20260414_000001_init;
mod m20260422_000001_seed_metrics;
mod m20260423_000001_seed_metrics_honest_nulls;
mod m20260428_000001_collab_metrics_update;
mod m20260429_000001_task_delivery_silver_rewrite;
mod m20260430_000001_update_git_bullet;
mod m20260507_000001_seed_crm_metrics;
mod m20260515_000001_task_delivery_bullet_rewrite;
mod m20260518_000001_collab_bullet_rewrite;
mod m20260519_000001_ai_bullet_rewrite;
mod m20260520_000001_code_quality_bullet_rewrite;
mod m20260522_000001_metric_catalog;
mod m20260522_000002_metric_threshold;
mod m20260522_000003_threshold_lock_audit;
mod m20260527_000001_seed_metric_catalog;
mod m20260527_000002_seed_metric_views;
mod m20260529_000001_metric_query_catalog_link;
mod m20260601_000001_ai_claude_team_metrics;
mod m20260601_000002_seed_claude_team_metrics_catalog;
mod m20260603_000001_seed_crm_metric_catalog;
mod m20260603_000002_link_crm_query_catalog;
mod m20260604_000001_task_delivery_bullet_distribution;
mod m20260604_000002_collab_bullet_distribution;
mod m20260604_000004_code_quality_bullet_distribution;
mod m20260604_000005_git_bullet_distribution;
mod m20260604_000006_ic_kpis_peer_median;
mod m20260604_000009_member_metric_values;
mod m20260606_000001_dept_metric_distributions;
mod m20260609_000001_ai_chatgpt_team_metrics;
mod m20260609_000002_seed_chatgpt_team_metrics_catalog;
mod m20260610_000001_fix_ai_label_drift;
mod m20260611_000001_ai_bullet_dept_reconcile;
mod m20260612_000001_support_metrics;
mod m20260612_000001_team_git_bullet;
mod m20260612_000002_seed_support_catalog;
mod m20260612_000003_link_support_query_catalog;
mod m20260618_000001_ai_claude_team_overage_metric;
mod m20260618_000002_seed_claude_team_overage_catalog;
mod m20260620_000001_seed_wiki_metrics;
mod m20260620_000002_seed_wiki_catalog;
mod m20260624_000001_collab_zulip_chat;
mod m20260624_000002_seed_zulip_collab_catalog;

#[cfg(test)]
mod live_tests;

use sea_orm_migration::prelude::*;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m20260414_000001_init::Migration),
            Box::new(m20260422_000001_seed_metrics::Migration),
            Box::new(m20260423_000001_seed_metrics_honest_nulls::Migration),
            Box::new(m20260428_000001_collab_metrics_update::Migration),
            Box::new(m20260429_000001_task_delivery_silver_rewrite::Migration),
            Box::new(m20260430_000001_update_git_bullet::Migration),
            Box::new(m20260507_000001_seed_crm_metrics::Migration),
            Box::new(m20260515_000001_task_delivery_bullet_rewrite::Migration),
            Box::new(m20260518_000001_collab_bullet_rewrite::Migration),
            Box::new(m20260519_000001_ai_bullet_rewrite::Migration),
            Box::new(m20260520_000001_code_quality_bullet_rewrite::Migration),
            Box::new(m20260522_000001_metric_catalog::Migration),
            Box::new(m20260522_000002_metric_threshold::Migration),
            Box::new(m20260522_000003_threshold_lock_audit::Migration),
            Box::new(m20260527_000001_seed_metric_catalog::Migration),
            Box::new(m20260527_000002_seed_metric_views::Migration),
            Box::new(m20260529_000001_metric_query_catalog_link::Migration),
            Box::new(m20260601_000001_ai_claude_team_metrics::Migration),
            Box::new(m20260601_000002_seed_claude_team_metrics_catalog::Migration),
            Box::new(m20260603_000001_seed_crm_metric_catalog::Migration),
            Box::new(m20260603_000002_link_crm_query_catalog::Migration),
            Box::new(m20260604_000001_task_delivery_bullet_distribution::Migration),
            Box::new(m20260604_000002_collab_bullet_distribution::Migration),
            Box::new(m20260604_000004_code_quality_bullet_distribution::Migration),
            Box::new(m20260604_000005_git_bullet_distribution::Migration),
            Box::new(m20260604_000006_ic_kpis_peer_median::Migration),
            Box::new(m20260604_000009_member_metric_values::Migration),
            Box::new(m20260606_000001_dept_metric_distributions::Migration),
            Box::new(m20260609_000001_ai_chatgpt_team_metrics::Migration),
            Box::new(m20260609_000002_seed_chatgpt_team_metrics_catalog::Migration),
            Box::new(m20260610_000001_fix_ai_label_drift::Migration),
            Box::new(m20260611_000001_ai_bullet_dept_reconcile::Migration),
            Box::new(m20260612_000001_support_metrics::Migration),
            Box::new(m20260612_000001_team_git_bullet::Migration),
            Box::new(m20260612_000002_seed_support_catalog::Migration),
            Box::new(m20260612_000003_link_support_query_catalog::Migration),
            Box::new(m20260618_000001_ai_claude_team_overage_metric::Migration),
            Box::new(m20260618_000002_seed_claude_team_overage_catalog::Migration),
            Box::new(m20260620_000001_seed_wiki_metrics::Migration),
            Box::new(m20260620_000002_seed_wiki_catalog::Migration),
            Box::new(m20260624_000001_collab_zulip_chat::Migration),
            Box::new(m20260624_000002_seed_zulip_collab_catalog::Migration),
        ]
    }
}

/// Per-table CHECK constraint names that the startup probe asserts present.
///
/// Source of truth for the probe in `infra/db/check_probe.rs`. Each entry maps
/// a table name to the CHECK names the corresponding migration emits. Keep in
/// sync with the `REQUIRED_CHECKS` const in each migration module.
pub const REQUIRED_CHECKS_BY_TABLE: &[(&str, &[&str])] = &[
    (
        "metric_catalog",
        m20260522_000001_metric_catalog::REQUIRED_CHECKS,
    ),
    (
        "metric_threshold",
        m20260522_000002_metric_threshold::REQUIRED_CHECKS,
    ),
    (
        "threshold_lock_audit",
        m20260522_000003_threshold_lock_audit::REQUIRED_CHECKS,
    ),
];

#[cfg(test)]
mod tests {
    use super::*;

    /// Guards against a developer adding a new catalog migration but
    /// forgetting to register its `REQUIRED_CHECKS` in
    /// [`REQUIRED_CHECKS_BY_TABLE`]. Catches the "silent probe degradation"
    /// failure mode where a missing CHECK at runtime would never be
    /// reported because the probe iterates this list.
    ///
    /// This test pins the catalog scope only — adding a non-catalog
    /// migration (e.g., another `analytics.metrics` rewrite) does not need
    /// a `REQUIRED_CHECKS_BY_TABLE` entry, so we don't enumerate all
    /// migrations. Tighten this test (or extract a per-migration
    /// `REQUIRED_CHECKS` registry trait) the day a fourth catalog
    /// migration lands.
    #[test]
    fn required_checks_by_table_covers_every_catalog_table() {
        // Wire each catalog migration's REQUIRED_CHECKS by its own const
        // pointer so renaming the const on one side fails compilation here
        // first — there is no string-matching slop in this comparison.
        let expected: &[(&str, &[&str])] = &[
            (
                "metric_catalog",
                m20260522_000001_metric_catalog::REQUIRED_CHECKS,
            ),
            (
                "metric_threshold",
                m20260522_000002_metric_threshold::REQUIRED_CHECKS,
            ),
            (
                "threshold_lock_audit",
                m20260522_000003_threshold_lock_audit::REQUIRED_CHECKS,
            ),
        ];

        for &(table, checks) in expected {
            let Some(&(_, registered)) =
                REQUIRED_CHECKS_BY_TABLE.iter().find(|&&(t, _)| t == table)
            else {
                panic!(
                    "catalog table `{table}` is not registered in \
                     REQUIRED_CHECKS_BY_TABLE — the startup probe will \
                     silently skip its CHECKs"
                );
            };
            assert_eq!(
                registered.as_ptr(),
                checks.as_ptr(),
                "REQUIRED_CHECKS_BY_TABLE entry for `{table}` points at a \
                 different REQUIRED_CHECKS slice than the migration module \
                 exports — keep them in sync"
            );
        }
    }
}
