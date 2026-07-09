//! Rewrite the Team / IC Bullet Collaboration `query_ref`s to consume the
//! new `collab_bullet_rows` shape (issue #433 §4.1, §4.3).
//!
//! Pairs with ingestion migration
//! `20260518000000_collab-bullet-rewrite.sql`, which drops the daily %
//! from the view for 2 ratio metrics and emits their raw counters
//! instead. The `query_ref`s now reconstruct the composites as
//! `Σnum / Σden` over the period — the only mathematically correct
//! period aggregation when daily denominators differ (CLAUDE.md
//! "Aggregation correctness").
//!
//! Mathematical changes:
//!   - `slack_msgs_per_active_day` avg(daily `total_chat_messages`, NULL-on-zero)
//!     → `Σslack_messages_sent` / `Σslack_active_days`
//!   - `slack_dm_ratio` avg(daily 100 * (total - channel) / total)
//!     → 100 * greatest(0, `Σslack_messages_sent` − `Σslack_channel_posts`) / `Σslack_messages_sent`
//!
//! The `greatest(0, …)` clamp on the numerator matches the silver-layer
//! convention from PR #266 / #431 (`direct_and_group_messages =
//! greatest(0, total - channel)`). Slack's Analytics API occasionally
//! returns `channel_posts > total_chat_messages` after deletions or
//! backfill — without the clamp the ratio would dip below 0 and break
//! the FE gauge scale.
//!
//! Preserved unchanged: all 18 raw counters (emails, meetings, chats,
//! documents, active-days) are period sums — same as the predecessor's
//! `multiIf(metric_key IN SUM_LIST, sum, avg)` branch resolved for them.
//! The `meeting_free` counter (1 per meeting-free day) is also a period
//! sum — count of meeting-free days in the window — and stays a single
//! raw key emitted by the view.
//!
//! Structural change:
//!   - Replaced `multiIf(metric_key=X, dispatch)` with wide-aggregate
//!     (`sumIf` per raw `metric_key` + composite-ratio formulas for the
//!     2 Slack ratios) + `ARRAY JOIN` unpivot back to long format.
//!     Mirrors the pattern used in
//!     `m20260515_000001_task_delivery_bullet_rewrite.rs`.
//!
//! Walker compatibility: each query has exactly two leaf subqueries that
//! read from `insight.collab_bullet_rows GROUP BY person_id` (one in
//! `p`, one in `inner_c`). `inject_date_filter_into_subqueries` in
//! `handlers.rs` walks both and injects `WHERE metric_date >= … AND <`
//! before the `GROUP BY` in each leaf — same behavior as before.
//!
//! Note on duplication: the wide-aggregate + ARRAY JOIN block is written
//! twice (once in `p`, once in `inner_c`). Intentional for this PR —
//! eliminating the dup requires CTE-style hoisting which conflicts with
//! the current `handlers.rs` parser. Tracked as issue #433 §3.4 for a
//! follow-up PR.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const TEAM_BULLET_COLLAB_ID: &str = "00000000000000000001000000000005";
const IC_BULLET_COLLAB_ID: &str = "00000000000000000001000000000012";

/// Inner wide-aggregate block: per-person resolved metrics for one row
/// per `person_id`, with every FE-visible `metric_key` materialized in
/// its own column. The 18 raw counters use `sumIf`; the 2 composite
/// ratios are computed as `Σnum / Σden` (with NULL on zero denominator,
/// so the outer `avg()` ignores undefined cases).
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

/// `ARRAY JOIN` unpivot: turns one wide `pp` row (with N metric columns)
/// into N long rows `(metric_key, v_period)`. The 18 raw counters + 2
/// composite ratios = 20 FE-visible `metric_key`s.
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

fn ic_query() -> String {
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

    /// Explicitly irreversible. The paired CH migration
    /// `20260518000000_collab-bullet-rewrite.sql` redefines
    /// `insight.collab_bullet_rows` to drop the 2 composite-ratio
    /// `metric_key`s and convert `metric_date` from `String` to `Date`.
    /// Restoring `metrics.query_ref` here without also reverting the
    /// view would leave the queries pointing at composite `metric_key`s
    /// the view no longer emits — the bullets would silently render
    /// `ComingSoon` in production. Roll back by reverting the paired CH
    /// migration first, then this `down()`. Same pattern as
    /// `m20260428_000001_collab_metrics_update`.
    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom(
            "m20260518_000001_collab_bullet_rewrite is irreversible: \
             roll back the paired CH migration 20260518000000_collab-bullet-rewrite.sql \
             (which drops slack_msgs_per_active_day / slack_dm_ratio from the view \
             and changes metric_date to Date) before reverting metrics.query_ref."
                .to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // String-contains tests — same rationale as
    // `m20260515_000001_task_delivery_bullet_rewrite::tests`.
    // Goal: catch the high-impact regressions a typo in this PR would
    // cause (silent NULL aggregation from a misspelled `metric_key`,
    // missing composite-ratio formula, `p`/`inner_c` shape drift).

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
    /// via `sumIf` must appear as a literal in the wide-aggregate. A
    /// typo here silently aggregates the column to NULL because no view
    /// row matches. This is the set of 18 raw keys emitted by the
    /// rewritten view (composite ratios are computed from these, not
    /// read directly).
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

        // Each side has its own GROUP BY person_id wide-aggregate.
        let person_groupbys = query.matches("GROUP BY person_id").count();
        assert_eq!(
            person_groupbys, 2,
            "{label}: expected 2 occurrences of `GROUP BY person_id` (p and inner_c), got {person_groupbys}"
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

        // Composite ratios must be present as formulas (not as `sumIf`
        // of themselves — that's the regression: forgetting the
        // composite split and reading the dropped `metric_key`).
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
        // Team-scope: company-wide median (not partitioned by org_unit_id).
        assert!(
            q.contains("company_median") && q.contains("company_min") && q.contains("company_max"),
            "team_query must expose company_* range, got:\n{q}"
        );
        assert!(
            !q.contains("team_median"),
            "team_query must NOT use team_median (that's the IC-side label)"
        );
        // Outer join key is metric_key only (no org_unit_id pairing).
        assert!(
            q.contains("ON c.metric_key = p.metric_key"),
            "team_query JOIN must be on metric_key alone"
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
    }
}
