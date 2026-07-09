//! Add the Zulip chat counter (`zulip_messages_sent`) to the Collaboration
//! bullet `query_ref`s (team `…0005`, IC `…0012`).
//!
//! Pairs with the ingestion-side gold view branch added to
//! `insight.collab_bullet_rows` (`20260518000000_collab-bullet-rewrite.sql`,
//! Branch 4b) which emits `zulip_messages_sent` for
//! `data_source = 'insight_zulip_proxy'` rows of
//! `silver.class_collab_chat_activity`, and the catalog row added in
//! `m20260624_000002_seed_zulip_collab_catalog`.
//!
//! Why a new migration rather than editing `m20260604_000002`: SeaORM
//! migrations are append-only (already applied in prod). This re-sets the two
//! collab bullet `query_ref`s to the `m20260604_000002` distribution shape
//! (value + median/range + p25/p75/n) with one extra raw counter
//! (`zulip_messages_sent`) wired through `wide_aggregate_pp` (`sumIf`) and the
//! `ARRAY JOIN` unpivot. All other keys are copied verbatim. `down()` restores
//! the `m20260604_000002` shape (without the Zulip key).
//!
//! Scope note: the per-person member-values (`…0041`) and per-department
//! distribution (`…0045`) collab `query_ref`s carry their own copies of the
//! same key list; if the Zulip card must also appear in the team heatmap /
//! department-distribution widgets, those need the same one-line additions in
//! follow-up migrations. The IC/Team bullet (this migration) is what backs the
//! person-profile Collaboration section card.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const TEAM_BULLET_COLLAB_ID: &str = "00000000000000000001000000000005";
const IC_BULLET_COLLAB_ID: &str = "00000000000000000001000000000012";

/// Inner wide-aggregate block — `m20260604_000002` verbatim PLUS the
/// `zulip_messages_sent` raw counter. One row per `person_id`, every
/// FE-visible `metric_key` in its own `_v` column.
fn wide_aggregate_pp() -> &'static str {
    "SELECT person_id, any(org_unit_id) AS org_unit_id, \
         sumIf(metric_value, metric_key = 'm365_emails_sent') AS m365_emails_sent_v, \
         sumIf(metric_value, metric_key = 'm365_emails_received') AS m365_emails_received_v, \
         sumIf(metric_value, metric_key = 'm365_emails_read') AS m365_emails_read_v, \
         sumIf(metric_value, metric_key = 'meeting_hours') AS meeting_hours_v, \
         sumIf(metric_value, metric_key = 'meetings_count') AS meetings_count_v, \
         sumIf(metric_value, metric_key = 'teams_meeting_hours') AS teams_meeting_hours_v, \
         sumIf(metric_value, metric_key = 'zoom_meeting_hours') AS zoom_meeting_hours_v, \
         sumIf(metric_value, metric_key = 'teams_meetings') AS teams_meetings_v, \
         sumIf(metric_value, metric_key = 'zoom_meetings') AS zoom_meetings_v, \
         sumIf(metric_value, metric_key = 'meeting_free') AS meeting_free_v, \
         sumIf(metric_value, metric_key = 'm365_teams_chats') AS m365_teams_chats_v, \
         sumIf(metric_value, metric_key = 'slack_messages_sent') AS slack_messages_sent_v, \
         sumIf(metric_value, metric_key = 'slack_channel_posts') AS slack_channel_posts_v, \
         sumIf(metric_value, metric_key = 'slack_active_days') AS slack_active_days_v, \
         sumIf(metric_value, metric_key = 'zulip_messages_sent') AS zulip_messages_sent_v, \
         sumIf(metric_value, metric_key = 'm365_files_shared_internal') AS m365_files_shared_internal_v, \
         sumIf(metric_value, metric_key = 'm365_files_shared_external') AS m365_files_shared_external_v, \
         sumIf(metric_value, metric_key = 'm365_files_engaged') AS m365_files_engaged_v, \
         sumIf(metric_value, metric_key = 'm365_active_days') AS m365_active_days_v, \
         if(sumIf(metric_value, metric_key = 'slack_active_days') > 0, \
            round(sumIf(metric_value, metric_key = 'slack_messages_sent') \
                  / sumIf(metric_value, metric_key = 'slack_active_days'), 1), \
            CAST(NULL AS Nullable(Float64))) AS slack_msgs_per_active_day_v, \
         if(sumIf(metric_value, metric_key = 'slack_messages_sent') > 0, \
            round(toFloat64(100) \
                  * greatest(toFloat64(0), \
                             sumIf(metric_value, metric_key = 'slack_messages_sent') \
                             - sumIf(metric_value, metric_key = 'slack_channel_posts')) \
                  / sumIf(metric_value, metric_key = 'slack_messages_sent'), 1), \
            CAST(NULL AS Nullable(Float64))) AS slack_dm_ratio_v \
     FROM insight.collab_bullet_rows \
     GROUP BY person_id"
}

/// `ARRAY JOIN` unpivot — `m20260604_000002` verbatim PLUS the
/// `zulip_messages_sent` tuple.
fn array_join_kv() -> &'static str {
    "ARRAY JOIN [ \
         ('m365_emails_sent',            m365_emails_sent_v), \
         ('m365_emails_received',        m365_emails_received_v), \
         ('m365_emails_read',            m365_emails_read_v), \
         ('meeting_hours',               meeting_hours_v), \
         ('meetings_count',              meetings_count_v), \
         ('teams_meeting_hours',         teams_meeting_hours_v), \
         ('zoom_meeting_hours',          zoom_meeting_hours_v), \
         ('teams_meetings',              teams_meetings_v), \
         ('zoom_meetings',               zoom_meetings_v), \
         ('meeting_free',                meeting_free_v), \
         ('m365_teams_chats',            m365_teams_chats_v), \
         ('slack_messages_sent',         slack_messages_sent_v), \
         ('slack_channel_posts',         slack_channel_posts_v), \
         ('slack_active_days',           slack_active_days_v), \
         ('zulip_messages_sent',         zulip_messages_sent_v), \
         ('m365_files_shared_internal',  m365_files_shared_internal_v), \
         ('m365_files_shared_external',  m365_files_shared_external_v), \
         ('m365_files_engaged',          m365_files_engaged_v), \
         ('m365_active_days',            m365_active_days_v), \
         ('slack_msgs_per_active_day',   slack_msgs_per_active_day_v), \
         ('slack_dm_ratio',              slack_dm_ratio_v) \
     ] AS kv"
}

/// Build a collab bullet `query_ref` (distribution shape: value + cohort
/// median/range + p25/p75/n) over a per-person/per-`metric_key` rollup.
/// `cohort_extra_group` is the cohort-side `GROUP BY` tail (`""` for the
/// company-wide team bullet, `", org_unit_id"` + join key for the IC bullet).
fn bullet_query(team_scoped: bool) -> String {
    let pp = wide_aggregate_pp();
    let kv = array_join_kv();
    if team_scoped {
        // Team bullet …0005: cohort = each member's own department, blended
        // headcount-weighted via avg(c.team_*). Verbatim m20260604_000002.
        format!(
            "SELECT p.metric_key AS metric_key, \
                    avg(p.v_period) AS value, \
                    avg(c.team_median) AS median, \
                    avg(c.team_min) AS range_min, \
                    avg(c.team_max) AS range_max, \
                    avg(c.team_p25) AS p25, \
                    avg(c.team_p75) AS p75, \
                    toFloat64(count(p.v_period)) AS n \
             FROM ( \
                 SELECT person_id, org_unit_id, \
                        kv.1 AS metric_key, kv.2 AS v_period \
                 FROM ({pp}) pp \
                 {kv} \
             ) p \
             LEFT JOIN ( \
                 SELECT metric_key, org_unit_id, \
                        quantileExact(0.5)(v_period) AS team_median, \
                        min(v_period) AS team_min, \
                        max(v_period) AS team_max, \
                        quantileExact(0.25)(v_period) AS team_p25, \
                        quantileExact(0.75)(v_period) AS team_p75, \
                        count(v_period) AS team_n \
                 FROM ( \
                     SELECT person_id, org_unit_id, \
                            kv.1 AS metric_key, kv.2 AS v_period \
                     FROM ({pp}) ppc \
                     {kv} \
                 ) inner_c \
                 GROUP BY metric_key, org_unit_id \
             ) c ON c.metric_key = p.metric_key AND c.org_unit_id = p.org_unit_id \
             GROUP BY p.metric_key"
        )
    } else {
        // IC bullet …0012: cohort = the person's own department. Verbatim
        // m20260604_000002.
        format!(
            "SELECT p.metric_key AS metric_key, \
                    avg(p.v_period) AS value, \
                    any(c.team_median) AS median, \
                    any(c.team_min) AS range_min, \
                    any(c.team_max) AS range_max, \
                    any(c.team_p25) AS p25, \
                    any(c.team_p75) AS p75, \
                    any(c.team_n) AS n \
             FROM ( \
                 SELECT person_id, org_unit_id, \
                        kv.1 AS metric_key, kv.2 AS v_period \
                 FROM ({pp}) pp \
                 {kv} \
             ) p \
             LEFT JOIN ( \
                 SELECT metric_key, org_unit_id, \
                        quantileExact(0.5)(v_period) AS team_median, \
                        min(v_period) AS team_min, \
                        max(v_period) AS team_max, \
                        quantileExact(0.25)(v_period) AS team_p25, \
                        quantileExact(0.75)(v_period) AS team_p75, \
                        count(v_period) AS team_n \
                 FROM ( \
                     SELECT person_id, org_unit_id, \
                            kv.1 AS metric_key, kv.2 AS v_period \
                     FROM ({pp}) ppc \
                     {kv} \
                 ) inner_c \
                 GROUP BY metric_key, org_unit_id \
             ) c ON c.metric_key = p.metric_key AND c.org_unit_id = p.org_unit_id \
             GROUP BY p.metric_key"
        )
    }
}

/// Predecessor (`m20260604_000002`) `query_ref`s — same as `bullet_query`
/// but WITHOUT the `zulip_messages_sent` key. Used by `down()`.
fn old_wide_aggregate_pp() -> &'static str {
    "SELECT person_id, any(org_unit_id) AS org_unit_id, \
         sumIf(metric_value, metric_key = 'm365_emails_sent') AS m365_emails_sent_v, \
         sumIf(metric_value, metric_key = 'm365_emails_received') AS m365_emails_received_v, \
         sumIf(metric_value, metric_key = 'm365_emails_read') AS m365_emails_read_v, \
         sumIf(metric_value, metric_key = 'meeting_hours') AS meeting_hours_v, \
         sumIf(metric_value, metric_key = 'meetings_count') AS meetings_count_v, \
         sumIf(metric_value, metric_key = 'teams_meeting_hours') AS teams_meeting_hours_v, \
         sumIf(metric_value, metric_key = 'zoom_meeting_hours') AS zoom_meeting_hours_v, \
         sumIf(metric_value, metric_key = 'teams_meetings') AS teams_meetings_v, \
         sumIf(metric_value, metric_key = 'zoom_meetings') AS zoom_meetings_v, \
         sumIf(metric_value, metric_key = 'meeting_free') AS meeting_free_v, \
         sumIf(metric_value, metric_key = 'm365_teams_chats') AS m365_teams_chats_v, \
         sumIf(metric_value, metric_key = 'slack_messages_sent') AS slack_messages_sent_v, \
         sumIf(metric_value, metric_key = 'slack_channel_posts') AS slack_channel_posts_v, \
         sumIf(metric_value, metric_key = 'slack_active_days') AS slack_active_days_v, \
         sumIf(metric_value, metric_key = 'm365_files_shared_internal') AS m365_files_shared_internal_v, \
         sumIf(metric_value, metric_key = 'm365_files_shared_external') AS m365_files_shared_external_v, \
         sumIf(metric_value, metric_key = 'm365_files_engaged') AS m365_files_engaged_v, \
         sumIf(metric_value, metric_key = 'm365_active_days') AS m365_active_days_v, \
         if(sumIf(metric_value, metric_key = 'slack_active_days') > 0, \
            round(sumIf(metric_value, metric_key = 'slack_messages_sent') \
                  / sumIf(metric_value, metric_key = 'slack_active_days'), 1), \
            CAST(NULL AS Nullable(Float64))) AS slack_msgs_per_active_day_v, \
         if(sumIf(metric_value, metric_key = 'slack_messages_sent') > 0, \
            round(toFloat64(100) \
                  * greatest(toFloat64(0), \
                             sumIf(metric_value, metric_key = 'slack_messages_sent') \
                             - sumIf(metric_value, metric_key = 'slack_channel_posts')) \
                  / sumIf(metric_value, metric_key = 'slack_messages_sent'), 1), \
            CAST(NULL AS Nullable(Float64))) AS slack_dm_ratio_v \
     FROM insight.collab_bullet_rows \
     GROUP BY person_id"
}

fn old_array_join_kv() -> &'static str {
    "ARRAY JOIN [ \
         ('m365_emails_sent',            m365_emails_sent_v), \
         ('m365_emails_received',        m365_emails_received_v), \
         ('m365_emails_read',            m365_emails_read_v), \
         ('meeting_hours',               meeting_hours_v), \
         ('meetings_count',              meetings_count_v), \
         ('teams_meeting_hours',         teams_meeting_hours_v), \
         ('zoom_meeting_hours',          zoom_meeting_hours_v), \
         ('teams_meetings',              teams_meetings_v), \
         ('zoom_meetings',               zoom_meetings_v), \
         ('meeting_free',                meeting_free_v), \
         ('m365_teams_chats',            m365_teams_chats_v), \
         ('slack_messages_sent',         slack_messages_sent_v), \
         ('slack_channel_posts',         slack_channel_posts_v), \
         ('slack_active_days',           slack_active_days_v), \
         ('m365_files_shared_internal',  m365_files_shared_internal_v), \
         ('m365_files_shared_external',  m365_files_shared_external_v), \
         ('m365_files_engaged',          m365_files_engaged_v), \
         ('m365_active_days',            m365_active_days_v), \
         ('slack_msgs_per_active_day',   slack_msgs_per_active_day_v), \
         ('slack_dm_ratio',              slack_dm_ratio_v) \
     ] AS kv"
}

fn old_bullet_query(team_scoped: bool) -> String {
    let pp = old_wide_aggregate_pp();
    let kv = old_array_join_kv();
    let agg = if team_scoped {
        "avg(c.team_median) AS median, avg(c.team_min) AS range_min, \
         avg(c.team_max) AS range_max, avg(c.team_p25) AS p25, \
         avg(c.team_p75) AS p75, toFloat64(count(p.v_period)) AS n"
    } else {
        "any(c.team_median) AS median, any(c.team_min) AS range_min, \
         any(c.team_max) AS range_max, any(c.team_p25) AS p25, \
         any(c.team_p75) AS p75, any(c.team_n) AS n"
    };
    format!(
        "SELECT p.metric_key AS metric_key, avg(p.v_period) AS value, {agg} \
         FROM ( \
             SELECT person_id, org_unit_id, kv.1 AS metric_key, kv.2 AS v_period \
             FROM ({pp}) pp {kv} \
         ) p \
         LEFT JOIN ( \
             SELECT metric_key, org_unit_id, \
                    quantileExact(0.5)(v_period) AS team_median, \
                    min(v_period) AS team_min, max(v_period) AS team_max, \
                    quantileExact(0.25)(v_period) AS team_p25, \
                    quantileExact(0.75)(v_period) AS team_p75, \
                    count(v_period) AS team_n \
             FROM ( \
                 SELECT person_id, org_unit_id, kv.1 AS metric_key, kv.2 AS v_period \
                 FROM ({pp}) ppc {kv} \
             ) inner_c \
             GROUP BY metric_key, org_unit_id \
         ) c ON c.metric_key = p.metric_key AND c.org_unit_id = p.org_unit_id \
         GROUP BY p.metric_key"
    )
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        for (hex_id, query) in [
            (TEAM_BULLET_COLLAB_ID, bullet_query(true)),
            (IC_BULLET_COLLAB_ID, bullet_query(false)),
        ] {
            db.execute_unprepared(&format!(
                "UPDATE metrics SET query_ref = '{qr}' WHERE id = UNHEX('{hex_id}')",
                qr = query.replace('\'', "''"),
            ))
            .await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        for (hex_id, query) in [
            (TEAM_BULLET_COLLAB_ID, old_bullet_query(true)),
            (IC_BULLET_COLLAB_ID, old_bullet_query(false)),
        ] {
            db.execute_unprepared(&format!(
                "UPDATE metrics SET query_ref = '{qr}' WHERE id = UNHEX('{hex_id}')",
                qr = query.replace('\'', "''"),
            ))
            .await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zulip_key_wired_into_pp_and_kv() {
        let pp = wide_aggregate_pp();
        assert!(
            pp.contains("metric_key = 'zulip_messages_sent') AS zulip_messages_sent_v"),
            "wide_aggregate_pp must sumIf the zulip key"
        );
        let kv = array_join_kv();
        assert!(
            kv.contains("('zulip_messages_sent',         zulip_messages_sent_v)"),
            "array_join_kv must unpivot the zulip key"
        );
    }

    #[test]
    fn down_shape_omits_zulip() {
        assert!(!old_wide_aggregate_pp().contains("zulip_messages_sent"));
        assert!(!old_array_join_kv().contains("zulip_messages_sent"));
    }

    #[test]
    fn ic_and_team_carry_distribution_columns() {
        for q in [bullet_query(true), bullet_query(false)] {
            for col in ["p25", "p75", " n ", "median", "range_min", "range_max"] {
                assert!(q.contains(col), "bullet query missing {col}");
            }
        }
    }
}
