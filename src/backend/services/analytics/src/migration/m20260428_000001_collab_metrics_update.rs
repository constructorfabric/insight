//! Collaboration metrics update — rename mislabeled keys, split mixed
//! metrics, add consistency / engagement metrics in the
//! `TEAM_BULLET_COLLAB` (UUID …05) and `IC_BULLET_COLLAB` (UUID …12)
//! bullet aggregation queries.
//!
//! Slack renames:
//!   `slack_message_engagement`   → `slack_messages_sent`
//!   `slack_thread_participation` → `slack_channel_posts`
//!
//! Microsoft 365 rename:
//!   `m365_teams_messages` → `m365_teams_chats`
//!
//! Microsoft 365 split:
//!   `m365_files_shared` → `m365_files_shared_internal` +
//!                         `m365_files_shared_external`
//!
//! New metrics added to the inner-aggregation `sum`-list:
//!   `slack_active_days`, `slack_channel_posts`, `slack_messages_sent`,
//!   `m365_emails_received`, `m365_emails_read`,
//!   `m365_files_shared_internal`, `m365_files_shared_external`,
//!   `m365_files_engaged`, `m365_active_days`, `m365_teams_chats`
//!
//! New metrics using default `avg`-aggregation (emitted as NULL when
//! the rate is undefined so CH `avg()` ignores them):
//!   `slack_msgs_per_active_day`, `slack_dm_ratio`
//!
//! Paired with CH migration `20260428000000_collab-metrics-update.sql`
//! which redefines `insight.collab_bullet_rows` to emit the renamed
//! keys, split metrics, and new metrics.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const TEAM_BULLET_COLLAB_ID: &str = "00000000000000000001000000000005";
const IC_BULLET_COLLAB_ID: &str = "00000000000000000001000000000012";

const SUM_LIST: &str = "'m365_emails_sent', 'm365_emails_received', 'm365_emails_read', 'm365_teams_chats', 'm365_files_shared_internal', 'm365_files_shared_external', 'm365_files_engaged', 'm365_active_days', 'meeting_hours', 'meetings_count', 'teams_meeting_hours', 'zoom_meeting_hours', 'teams_meetings', 'zoom_meetings', 'meeting_free', 'slack_messages_sent', 'slack_channel_posts', 'slack_active_days'";

fn team_query() -> String {
    format!(
        "SELECT p.metric_key AS metric_key, avg(p.v_period) AS value, any(c.company_median) AS median, any(c.company_min) AS range_min, any(c.company_max) AS range_max FROM (SELECT metric_key, person_id, any(org_unit_id) AS org_unit_id, multiIf(metric_key IN ({SUM_LIST}), sum(metric_value), avg(metric_value)) AS v_period FROM insight.collab_bullet_rows GROUP BY metric_key, person_id) p LEFT JOIN (SELECT metric_key, quantileExact(0.5)(v_period) AS company_median, min(v_period) AS company_min, max(v_period) AS company_max FROM (SELECT metric_key, person_id, multiIf(metric_key IN ({SUM_LIST}), sum(metric_value), avg(metric_value)) AS v_period FROM insight.collab_bullet_rows GROUP BY metric_key, person_id) inner_c GROUP BY metric_key) c ON c.metric_key = p.metric_key GROUP BY p.metric_key"
    )
}

fn ic_query() -> String {
    format!(
        "SELECT p.metric_key AS metric_key, avg(p.v_period) AS value, any(c.team_median) AS median, any(c.team_min) AS range_min, any(c.team_max) AS range_max FROM (SELECT metric_key, person_id, any(org_unit_id) AS org_unit_id, multiIf(metric_key IN ({SUM_LIST}), sum(metric_value), avg(metric_value)) AS v_period FROM insight.collab_bullet_rows GROUP BY metric_key, person_id) p LEFT JOIN (SELECT metric_key, org_unit_id, quantileExact(0.5)(v_period) AS team_median, min(v_period) AS team_min, max(v_period) AS team_max FROM (SELECT metric_key, person_id, any(org_unit_id) AS org_unit_id, multiIf(metric_key IN ({SUM_LIST}), sum(metric_value), avg(metric_value)) AS v_period FROM insight.collab_bullet_rows GROUP BY metric_key, person_id) inner_c GROUP BY metric_key, org_unit_id) c ON c.metric_key = p.metric_key AND c.org_unit_id = p.org_unit_id GROUP BY p.metric_key"
    )
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();

        for (hex_id, query) in [
            (TEAM_BULLET_COLLAB_ID, team_query()),
            (IC_BULLET_COLLAB_ID, ic_query()),
        ] {
            let qr = query.replace('\'', "''");
            db.execute_unprepared(&format!(
                "UPDATE metrics SET query_ref = '{qr}' WHERE id = UNHEX('{hex_id}')"
            ))
            .await?;
        }

        Ok(())
    }

    /// Explicitly irreversible. The paired CH migration
    /// `20260428000000_collab-metrics-update.sql` redefines
    /// `insight.collab_bullet_rows` to emit a different `metric_key` set.
    /// Restoring `metrics.query_ref` here without also reverting the view
    /// would leave the queries pointing at `metric_keys` the view no longer
    /// emits — the bullets would silently render `ComingSoon` in
    /// production. Roll back by re-running the previous CH migration
    /// (`20260427120000_views-from-silver.sql`) first, then this `down()`.
    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom(
            "m20260428_000001_collab_metrics_update is irreversible: \
             roll back the paired CH migration 20260428000000_collab-metrics-update.sql \
             (re-run 20260427120000_views-from-silver.sql) before reverting \
             metrics.query_ref."
                .to_string(),
        ))
    }
}
