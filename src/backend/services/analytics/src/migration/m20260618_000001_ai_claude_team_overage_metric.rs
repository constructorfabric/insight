//! Add Claude Team `cc_overage` to Team / IC Bullet AI `query_ref`s
//! (per-seat spend over the monthly credit limit).
//!
//! Pairs with ingestion migration
//! `20260618000000_ai-claude-team-overage-gold.sql`, which adds Branch 6
//! (`metric_key = 'cc_overage'`) to `insight.ai_bullet_rows`, emitting
//! `overage_cents` from `silver.class_ai_overage` (source='`claude_team`').
//!
//! Changes to each `query_ref` (Team + IC), extending the m20260609 head:
//!   1. New `cc_overage_v` in the wide-aggregate — honest-NULL guarded
//!      (`if(countIf(key) > 0, sumIf(value), NULL)`), like the prs metrics:
//!      a seat with no overage reading renders `ComingSoon`, while a seat
//!      within its limit emits a real `0` (the gold view emits 0 only when
//!      a limit is known).
//!   2. New `('cc_overage', cc_overage_v)` entry in the ARRAY JOIN unpivot.
//!
//! `cc_overage` is a plain counter (cents over limit), NOT an active-marker —
//! it is NOT added to `ACTIVE_LIST`, so the outer dispatch aggregates it with
//! `avg(v_period)` (average per-person overage over the period), identical to
//! `cc_cost`.
//!
//! Backend-emitted `metric_key`s: 22 → 23.
//! Catalog metadata (label / unit '¢' / lower-is-better / thresholds) is
//! seeded by the paired migration
//! `m20260618_000002_seed_claude_team_overage_catalog`. FE renders it once a
//! `bullet-layout-groups` entry is added in `cyber-insight-front`.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const TEAM_BULLET_AI_ID: &str = "00000000000000000001000000000006";
const IC_BULLET_AI_ID: &str = "00000000000000000001000000000013";

/// Active-marker `metric_key`s — unchanged from m20260609. `cc_overage` is a
/// counter, NOT an active-marker, so it is deliberately absent here.
const ACTIVE_LIST: &str =
    "'active_ai_members', 'cursor_active', 'cc_active', 'codex_active', 'chatgpt_active'";

/// Inner wide-aggregate: one row per `person_id`, every FE-visible
/// `metric_key` in its own column. Extends m20260609 with `cc_overage_v`.
fn wide_aggregate_pp() -> &'static str {
    "SELECT person_id, any(org_unit_id) AS org_unit_id, \
         if(countIf(metric_key = 'active_ai_members') > 0, toFloat64(1), CAST(NULL AS Nullable(Float64))) AS active_ai_members_v, \
         if(countIf(metric_key = 'cursor_active') > 0, toFloat64(1), CAST(NULL AS Nullable(Float64))) AS cursor_active_v, \
         if(countIf(metric_key = 'cc_active') > 0, toFloat64(1), CAST(NULL AS Nullable(Float64))) AS cc_active_v, \
         sumIf(metric_value, metric_key = 'cursor_completions') AS cursor_completions_v, \
         sumIf(metric_value, metric_key = 'cursor_agents') AS cursor_agents_v, \
         sumIf(metric_value, metric_key = 'cursor_lines') AS cursor_lines_v, \
         sumIf(metric_value, metric_key = 'cc_sessions') AS cc_sessions_v, \
         sumIf(metric_value, metric_key = 'cc_lines') AS cc_lines_v, \
         sumIf(metric_value, metric_key = 'cc_tool_accept') AS cc_tool_accept_v, \
         sumIf(metric_value, metric_key = 'team_ai_loc') AS team_ai_loc_v, \
         sumIf(metric_value, metric_key = 'cc_cost') AS cc_cost_v, \
         if(countIf(metric_key = 'prs_with_cc') > 0, sumIf(metric_value, metric_key = 'prs_with_cc'), CAST(NULL AS Nullable(Float64))) AS prs_with_cc_v, \
         if(countIf(metric_key = 'prs_total') > 0, sumIf(metric_value, metric_key = 'prs_total'), CAST(NULL AS Nullable(Float64))) AS prs_total_v, \
         if(sumIf(metric_value, metric_key = 'cursor_offered') > 0, \
            round(toFloat64(100) \
                  * sumIf(metric_value, metric_key = 'cursor_completions') \
                  / sumIf(metric_value, metric_key = 'cursor_offered'), 1), \
            CAST(NULL AS Nullable(Float64))) AS cursor_acceptance_v, \
         if(sumIf(metric_value, metric_key = 'cc_offered') > 0, \
            round(toFloat64(100) \
                  * sumIf(metric_value, metric_key = 'cc_tool_accept') \
                  / sumIf(metric_value, metric_key = 'cc_offered'), 1), \
            CAST(NULL AS Nullable(Float64))) AS cc_tool_acceptance_v, \
         if(sumIf(metric_value, metric_key = 'cursor_total_lines') > 0, \
            round(toFloat64(100) \
                  * sumIf(metric_value, metric_key = 'cursor_lines') \
                  / sumIf(metric_value, metric_key = 'cursor_total_lines'), 1), \
            CAST(NULL AS Nullable(Float64))) AS ai_loc_share2_v, \
         if(countIf(metric_key = 'codex_active') > 0, toFloat64(1), CAST(NULL AS Nullable(Float64))) AS codex_active_v, \
         sumIf(metric_value, metric_key = 'chatgpt') AS chatgpt_v, \
         CAST(NULL AS Nullable(Float64)) AS claude_web_v, \
         sumIf(metric_value, metric_key = 'codex_lines') AS codex_lines_v, \
         sumIf(metric_value, metric_key = 'codex_sessions') AS codex_sessions_v, \
         if(countIf(metric_key = 'chatgpt_active') > 0, toFloat64(1), CAST(NULL AS Nullable(Float64))) AS chatgpt_active_v, \
         if(countIf(metric_key = 'cc_overage') > 0, sumIf(metric_value, metric_key = 'cc_overage'), CAST(NULL AS Nullable(Float64))) AS cc_overage_v \
     FROM insight.ai_bullet_rows \
     GROUP BY person_id"
}

/// `ARRAY JOIN` unpivot: wide columns → long rows per person.
/// 22 keys from m20260609 + `cc_overage` = 23 total.
fn array_join_kv() -> &'static str {
    "ARRAY JOIN [ \
         ('active_ai_members',  active_ai_members_v), \
         ('cursor_active',      cursor_active_v), \
         ('cc_active',          cc_active_v), \
         ('cursor_completions', cursor_completions_v), \
         ('cursor_agents',      cursor_agents_v), \
         ('cursor_lines',       cursor_lines_v), \
         ('cc_sessions',        cc_sessions_v), \
         ('cc_lines',           cc_lines_v), \
         ('cc_tool_accept',     cc_tool_accept_v), \
         ('team_ai_loc',        team_ai_loc_v), \
         ('cc_cost',            cc_cost_v), \
         ('prs_with_cc',        prs_with_cc_v), \
         ('prs_total',          prs_total_v), \
         ('cursor_acceptance',  cursor_acceptance_v), \
         ('cc_tool_acceptance', cc_tool_acceptance_v), \
         ('ai_loc_share2',      ai_loc_share2_v), \
         ('codex_active',       codex_active_v), \
         ('chatgpt',            chatgpt_v), \
         ('claude_web',         claude_web_v), \
         ('codex_lines',        codex_lines_v), \
         ('codex_sessions',     codex_sessions_v), \
         ('chatgpt_active',     chatgpt_active_v), \
         ('cc_overage',         cc_overage_v) \
     ] AS kv"
}

fn team_query() -> String {
    let pp = wide_aggregate_pp();
    let kv = array_join_kv();
    format!(
        "SELECT p.metric_key AS metric_key, \
                multiIf(p.metric_key IN ({ACTIVE_LIST}), sum(p.v_period), avg(p.v_period)) AS value, \
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
                    multiIf(metric_key IN ({ACTIVE_LIST}), \
                            if(count(v_period) = 0, CAST(NULL AS Nullable(Float64)), toFloat64(0)), \
                            quantileExact(0.5)(v_period)) AS company_median, \
                    multiIf(metric_key IN ({ACTIVE_LIST}), \
                            if(count(v_period) = 0, CAST(NULL AS Nullable(Float64)), toFloat64(0)), \
                            min(v_period)) AS company_min, \
                    multiIf(metric_key IN ({ACTIVE_LIST}), \
                            if(count(v_period) = 0, CAST(NULL AS Nullable(Float64)), toFloat64(count())), \
                            max(v_period)) AS company_max \
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
                multiIf(p.metric_key IN ({ACTIVE_LIST}), sum(p.v_period), avg(p.v_period)) AS value, \
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
                    multiIf(metric_key IN ({ACTIVE_LIST}), \
                            if(count(v_period) = 0, CAST(NULL AS Nullable(Float64)), toFloat64(0)), \
                            quantileExact(0.5)(v_period)) AS team_median, \
                    multiIf(metric_key IN ({ACTIVE_LIST}), \
                            if(count(v_period) = 0, CAST(NULL AS Nullable(Float64)), toFloat64(0)), \
                            min(v_period)) AS team_min, \
                    multiIf(metric_key IN ({ACTIVE_LIST}), \
                            if(count(v_period) = 0, CAST(NULL AS Nullable(Float64)), toFloat64(count())), \
                            max(v_period)) AS team_max \
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
            (TEAM_BULLET_AI_ID, team_query()),
            (IC_BULLET_AI_ID, ic_query()),
        ] {
            db.execute_unprepared(&format!(
                "UPDATE metrics SET query_ref = '{qr}' WHERE id = UNHEX('{hex_id}')",
                qr = query.replace('\'', "''"),
            ))
            .await?;
        }
        Ok(())
    }

    /// Irreversible — roll back the paired CH migration
    /// `20260618000000_ai-claude-team-overage-gold.sql` first (which removes
    /// the `cc_overage` branch from the view), then restore the previous
    /// `query_ref` from `m20260609_000001` manually.
    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom(
            "m20260618_000001_ai_claude_team_overage_metric is irreversible: \
             roll back the paired CH migration \
             20260618000000_ai-claude-team-overage-gold.sql first."
                .to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// All 23 FE-visible `metric_keys` must appear in the ARRAY JOIN unpivot
    /// (22 from m20260609 + `cc_overage`).
    const EXPECTED_METRIC_KEYS: &[&str] = &[
        "active_ai_members",
        "cursor_active",
        "cc_active",
        "cursor_completions",
        "cursor_agents",
        "cursor_lines",
        "cc_sessions",
        "cc_lines",
        "cc_tool_accept",
        "team_ai_loc",
        "cc_cost",
        "prs_with_cc",
        "prs_total",
        "cursor_acceptance",
        "cc_tool_acceptance",
        "ai_loc_share2",
        "codex_active",
        "chatgpt",
        "claude_web",
        "codex_lines",
        "codex_sessions",
        "chatgpt_active",
        "cc_overage",
    ];

    #[test]
    fn array_join_emits_all_23_keys() {
        let kv = array_join_kv();
        for key in EXPECTED_METRIC_KEYS {
            assert!(
                kv.contains(&format!("('{key}',")),
                "ARRAY JOIN missing key {key}"
            );
        }
        assert_eq!(
            kv.matches("('").count(),
            23,
            "ARRAY JOIN must emit exactly 23 keys"
        );
        assert_eq!(EXPECTED_METRIC_KEYS.len(), 23);
    }

    /// `cc_overage` is a cost counter → must NOT be in `ACTIVE_LIST`
    /// (it is averaged per person, not counted as a DAU marker).
    #[test]
    fn cc_overage_is_not_active_marker() {
        assert!(
            !ACTIVE_LIST.contains("'cc_overage'"),
            "cc_overage is a counter, not an active marker"
        );
    }

    /// `cc_overage` must be honest-NULL guarded (countIf > 0 … else NULL),
    /// like the prs metrics: no reading → `ComingSoon`, never a fake 0.
    #[test]
    fn cc_overage_is_honest_null_guarded() {
        let pp = wide_aggregate_pp();
        // countIf guard present, followed by the sumIf and the NULL-as alias.
        assert!(
            pp.contains("if(countIf(metric_key = 'cc_overage') > 0"),
            "cc_overage_v must be countIf-guarded (honest-NULL), not a bare sumIf"
        );
        assert!(
            pp.contains("sumIf(metric_value, metric_key = 'cc_overage')"),
            "cc_overage_v must read its value via sumIf"
        );
        assert!(
            pp.contains("AS cc_overage_v"),
            "cc_overage_v alias must be present in the wide-aggregate"
        );
    }

    /// Both `query_refs` must embed the new key end-to-end.
    #[test]
    fn queries_reference_cc_overage() {
        for q in [team_query(), ic_query()] {
            assert!(q.contains("cc_overage"), "query missing cc_overage");
            assert!(q.contains("ARRAY JOIN"), "query missing ARRAY JOIN unpivot");
        }
    }

    /// Sanity: the m20260609 keys are still present (no regression).
    #[test]
    fn prior_keys_preserved() {
        let kv = array_join_kv();
        for key in ["cc_cost", "codex_lines", "chatgpt_active", "team_ai_loc"] {
            assert!(kv.contains(&format!("('{key}',")), "regression: lost {key}");
        }
    }
}
