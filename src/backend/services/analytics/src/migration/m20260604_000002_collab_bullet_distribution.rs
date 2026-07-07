//! Seed the Collaboration bullets (team `…0005`, IC `…0012`): per-person
//! wide-aggregate over `insight.collab_bullet_rows` with cohort distribution
//! (p25/p50/p75 + size `n`). IC compares against the person's DEPARTMENT
//! cohort (`any(c.team_*)`, grouped by `org_unit_id`); the team bullet blends
//! each roster member's department cohort (`avg(c.team_*)`, headcount-weighted).
//!
//! Roster scoping is done by the handler's `person_id IN (roster)` filter, so
//! the team query keeps `GROUP BY p.metric_key` — no supervisor join. Both
//! leaves `GROUP BY person_id`, so `inject_date_filter_into_subqueries`
//! injects the `metric_date` range before each GROUP BY.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const TEAM_BULLET_COLLAB_ID: &str = "00000000000000000001000000000005";
const IC_BULLET_COLLAB_ID: &str = "00000000000000000001000000000012";

/// Inner wide-aggregate block (copied verbatim from
/// `m20260518_000001_collab_bullet_rewrite::wide_aggregate_pp`): per-person
/// resolved metrics, one row per `person_id`, every FE-visible `metric_key`
/// in its own column. The 18 raw counters use `sumIf`; the 2 composite
/// ratios are computed as `Σnum / Σden` (NULL on zero denominator). Used
/// by both sides of both queries.
///
/// `pp` is the output alias used by the caller.
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

/// `ARRAY JOIN` unpivot (copied verbatim from
/// `m20260518_000001_collab_bullet_rewrite::array_join_kv`): turns one wide
/// `pp` row (with N metric columns) into N long rows
/// `(metric_key, v_period)`. 18 raw counters + 2 composite ratios = 20
/// FE-visible `metric_key`s.
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
         ('m365_files_shared_internal',  m365_files_shared_internal_v), \
         ('m365_files_shared_external',  m365_files_shared_external_v), \
         ('m365_files_engaged',          m365_files_engaged_v), \
         ('m365_active_days',            m365_active_days_v), \
         ('slack_msgs_per_active_day',   slack_msgs_per_active_day_v), \
         ('slack_dm_ratio',              slack_dm_ratio_v) \
     ] AS kv"
}

fn team_query() -> String {
    let pp = wide_aggregate_pp();
    let kv = array_join_kv();
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
}

fn ic_query() -> String {
    let pp = wide_aggregate_pp();
    let kv = array_join_kv();
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

/// Predecessor `query_ref`s as set by
/// `m20260518_000001_collab_bullet_rewrite` (the current production shape:
/// wide-aggregate + ARRAY JOIN, no distribution columns, no supervisor
/// scope). Restored by `down()` so a rollback returns to that shape rather
/// than the long-obsolete `m20260428` seed. Copied verbatim from
/// m20260518's `team_query()` / `ic_query()` output.
fn old_team_query() -> String {
    let pp = wide_aggregate_pp();
    let kv = array_join_kv();
    format!(
        "SELECT p.metric_key AS metric_key, \
                avg(p.v_period) AS value, \
                any(c.company_median) AS median, \
                any(c.company_min) AS range_min, \
                any(c.company_max) AS range_max \
         FROM ( \
             SELECT person_id, org_unit_id, \
                    kv.1 AS metric_key, kv.2 AS v_period \
             FROM ({pp}) pp \
             {kv} \
         ) p \
         LEFT JOIN ( \
             SELECT metric_key, \
                    quantileExact(0.5)(v_period) AS company_median, \
                    min(v_period) AS company_min, \
                    max(v_period) AS company_max \
             FROM ( \
                 SELECT kv.1 AS metric_key, kv.2 AS v_period \
                 FROM ({pp}) ppc \
                 {kv} \
             ) inner_c \
             GROUP BY metric_key \
         ) c ON c.metric_key = p.metric_key \
         GROUP BY p.metric_key"
    )
}

fn old_ic_query() -> String {
    let pp = wide_aggregate_pp();
    let kv = array_join_kv();
    format!(
        "SELECT p.metric_key AS metric_key, \
                avg(p.v_period) AS value, \
                any(c.team_median) AS median, \
                any(c.team_min) AS range_min, \
                any(c.team_max) AS range_max \
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
                    max(v_period) AS team_max \
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

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        for (hex_id, query) in [
            (TEAM_BULLET_COLLAB_ID, team_query()),
            (IC_BULLET_COLLAB_ID, ic_query()),
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
            (TEAM_BULLET_COLLAB_ID, old_team_query()),
            (IC_BULLET_COLLAB_ID, old_ic_query()),
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

    // String-contains tests — same rationale as
    // `m20260518_000001_collab_bullet_rewrite::tests`. Goal: catch the
    // high-impact regressions a typo in this PR would cause (silent NULL
    // aggregation from a misspelled `metric_key`, missing composite-ratio
    // formula, `p`/`inner_c` shape drift, missing distribution columns).

    /// Every FE-visible `metric_key` the bullet section emits must appear
    /// as an `('X', X_v)` entry in the ARRAY JOIN unpivot.
    const EXPECTED_METRIC_KEYS: &[&str] = &[
        "m365_emails_sent",
        "m365_emails_received",
        "m365_emails_read",
        "meeting_hours",
        "meetings_count",
        "teams_meeting_hours",
        "zoom_meeting_hours",
        "teams_meetings",
        "zoom_meetings",
        "meeting_free",
        "m365_teams_chats",
        "slack_messages_sent",
        "slack_channel_posts",
        "slack_active_days",
        "m365_files_shared_internal",
        "m365_files_shared_external",
        "m365_files_engaged",
        "m365_active_days",
        "slack_msgs_per_active_day",
        "slack_dm_ratio",
    ];

    /// Every raw `metric_key` the view emits that the `query_ref` reads
    /// via `sumIf` must appear as a literal in the wide-aggregate. A typo
    /// here silently aggregates the column to NULL because no view row
    /// matches. This is the set of 18 raw keys emitted by the rewritten
    /// view (composite ratios are computed from these, not read directly).
    const EXPECTED_RAW_KEYS_READ_BY_QUERY: &[&str] = &[
        "m365_emails_sent",
        "m365_emails_received",
        "m365_emails_read",
        "meeting_hours",
        "meetings_count",
        "teams_meeting_hours",
        "zoom_meeting_hours",
        "teams_meetings",
        "zoom_meetings",
        "meeting_free",
        "m365_teams_chats",
        "slack_messages_sent",
        "slack_channel_posts",
        "slack_active_days",
        "m365_files_shared_internal",
        "m365_files_shared_external",
        "m365_files_engaged",
        "m365_active_days",
    ];

    fn assert_query_shape(query: &str, label: &str) {
        // Both sides of the JOIN read from the same source table.
        let table_refs = query.matches("insight.collab_bullet_rows").count();
        assert_eq!(
            table_refs, 2,
            "{label}: expected 2 references to `insight.collab_bullet_rows` (one per JOIN side, no CTE hoist yet — see issue #433 §3.4), got {table_refs}"
        );

        // Each side has its own GROUP BY <person_id> wide-aggregate. The
        // Team `p` side qualifies it as `r.person_id` (people join present);
        // the company / IC sides use a bare `person_id`. Count both forms.
        let person_groupbys = query.matches("GROUP BY person_id").count()
            + query.matches("GROUP BY r.person_id").count();
        assert_eq!(
            person_groupbys, 2,
            "{label}: expected 2 per-person wide-aggregate GROUP BYs (p and inner_c), got {person_groupbys}"
        );

        // FE-visible metric_keys are unpivoted via ARRAY JOIN.
        for key in EXPECTED_METRIC_KEYS {
            let literal = format!("'{key}'");
            assert!(
                query.contains(&literal),
                "{label}: missing FE-visible metric_key literal {literal} in ARRAY JOIN unpivot"
            );
        }

        // Raw metric_keys the wide-aggregate reads from the view must
        // match what the view emits. A typo here = silent NULL.
        for key in EXPECTED_RAW_KEYS_READ_BY_QUERY {
            let read = format!("metric_key = '{key}'");
            assert!(
                query.contains(&read),
                "{label}: missing read of raw metric_key {key} (`metric_key = '{key}'`) in wide-aggregate"
            );
        }

        // Composite ratios must be present as formulas (not as `sumIf` of
        // themselves — that's the regression: forgetting the composite
        // split and reading the dropped `metric_key`).
        assert!(
            query.contains("slack_msgs_per_active_day_v"),
            "{label}: missing slack_msgs_per_active_day_v composite formula"
        );
        assert!(
            query.contains("slack_dm_ratio_v"),
            "{label}: missing slack_dm_ratio_v composite formula"
        );
        // The composite keys must NOT be read via `sumIf` — the view no
        // longer emits them, so a `metric_key = 'slack_msgs_per_active_day'`
        // read would aggregate to NULL.
        assert!(
            !query.contains("metric_key = 'slack_msgs_per_active_day'"),
            "{label}: composite slack_msgs_per_active_day must not be read via sumIf — it's no longer emitted by the view"
        );
        assert!(
            !query.contains("metric_key = 'slack_dm_ratio'"),
            "{label}: composite slack_dm_ratio must not be read via sumIf — it's no longer emitted by the view"
        );

        // slack_dm_ratio numerator must be clamped via greatest(0, …).
        // Slack Analytics API occasionally returns channel_posts >
        // total_chat_messages after deletions/backfill, which would
        // produce a negative ratio and break the FE gauge. Silver layer
        // (PR #266/#431) applies the same clamp to
        // direct_and_group_messages.
        let Some(dm_start) = query.find("slack_dm_ratio_v") else {
            panic!("{label}: slack_dm_ratio_v not found");
        };
        let formula_start = dm_start.saturating_sub(400);
        let formula_window = &query[formula_start..dm_start];
        assert!(
            formula_window.contains("greatest(toFloat64(0)"),
            "{label}: slack_dm_ratio_v numerator must be clamped via greatest(toFloat64(0), …); got:\n{formula_window}"
        );
    }

    #[test]
    fn team_query_shape() {
        let q = team_query();
        assert_query_shape(&q, "team_query");
        // Team bullet blends each roster member's DEPARTMENT cohort:
        // avg(c.team_*) joined on org_unit_id (headcount-weighted), not the
        // old company-wide any(c.company_*).
        assert!(
            q.contains("c.org_unit_id = p.org_unit_id"),
            "team_query must join the department cohort on org_unit_id, got:\n{q}"
        );
        for col in ["median", "min", "max", "p25", "p75"] {
            let blended = format!("avg(c.team_{col})");
            assert!(
                q.contains(&blended),
                "team_query cohort column must blend via `{blended}`, got:\n{q}"
            );
        }
        assert!(
            !q.contains("any(c.team_") && !q.contains("company_median"),
            "team_query must NOT use any(c.team_…) or company_* labels, got:\n{q}"
        );
        assert!(
            q.contains("avg(p.v_period) AS value"),
            "team_query value must stay avg(p.v_period), got:\n{q}"
        );
        assert!(
            q.contains("quantileExact(0.25)(v_period) AS team_p25")
                && q.contains("quantileExact(0.75)(v_period) AS team_p75")
                && q.contains("count(v_period) AS team_n"),
            "team_query cohort must compute team_p25 / team_p75 / team_n, got:\n{q}"
        );
        assert!(
            q.contains("toFloat64(count(p.v_period)) AS n"),
            "team_query cohort size must be toFloat64(count(p.v_period)) AS n, got:\n{q}"
        );
        assert!(
            q.contains("GROUP BY metric_key, org_unit_id"),
            "team_query cohort must group per metric_key, org_unit_id, got:\n{q}"
        );

        // Roster scoping happens at the handler (`person_id IN (...)`), so the
        // outer groups by metric_key — no supervisor join.
        assert!(
            q.contains("GROUP BY p.metric_key"),
            "team_query outer GROUP BY must be metric_key, got:\n{q}"
        );
        assert!(
            !q.contains("supervisor_email") && !q.contains("insight.people"),
            "team_query must NOT join insight.people / supervisor_email, got:\n{q}"
        );
    }

    #[test]
    fn ic_query_shape() {
        let q = ic_query();
        assert_query_shape(&q, "ic_query");
        // IC-scope: team-wide median (partitioned by org_unit_id).
        assert!(
            q.contains("team_median") && q.contains("team_min") && q.contains("team_max"),
            "ic_query must expose team_* range, got:\n{q}"
        );
        assert!(
            !q.contains("company_median"),
            "ic_query must NOT use company_median (that's the Team-side label)"
        );
        // Outer join key includes org_unit_id.
        assert!(
            q.contains("c.org_unit_id = p.org_unit_id"),
            "ic_query JOIN must include org_unit_id"
        );

        // Distribution: team-wide P25 / P75 / cohort size, surfaced as
        // p25 / p75 / n on the outer SELECT.
        assert!(
            q.contains("team_p25") && q.contains("team_p75") && q.contains("team_n"),
            "ic_query must compute team_p25 / team_p75 / team_n, got:\n{q}"
        );
        assert!(
            q.contains("quantileExact(0.25)(v_period) AS team_p25"),
            "ic_query team_p25 must be quantileExact(0.25)"
        );
        assert!(
            q.contains("quantileExact(0.75)(v_period) AS team_p75"),
            "ic_query team_p75 must be quantileExact(0.75)"
        );
        assert!(
            q.contains("count(v_period) AS team_n"),
            "ic_query team_n must be count(v_period)"
        );
        assert!(
            q.contains("AS p25") && q.contains("AS p75") && q.contains("AS n"),
            "ic_query outer SELECT must surface p25 / p75 / n, got:\n{q}"
        );

        // IC query is NOT supervisor-scoped — no people join, no
        // supervisor_email grouping.
        assert!(
            !q.contains("supervisor_email"),
            "ic_query must NOT reference supervisor_email (team-scope is via org_unit_id)"
        );
        assert!(
            !q.contains("insight.people"),
            "ic_query must NOT join insight.people"
        );
    }
}
