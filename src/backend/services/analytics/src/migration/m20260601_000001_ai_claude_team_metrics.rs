//! Extend Team / IC Bullet AI `query_ref`s with Claude Team metrics
//! (INSIGHT-458).
//!
//! Pairs with ingestion migration
//! `20260601000000_ai-claude-team-metrics.sql`, which adds three new
//! `metric_key`s to Branch 3 (`tool = 'claude_code'`) of
//! `insight.ai_bullet_rows`:
//!
//!   `cc_cost`      — per-user-per-day cost in cents (`cost_cents`).
//!                    Claude Team is the only source at this grain;
//!                    all other `claude_code` rows contribute 0 via
//!                    COALESCE, so the `sumIf` is additive-safe.
//!   `prs_with_cc`  — PRs where Claude Code was active at least once
//!                    (`prs_with_cc_count`). Populated only on tenants
//!                    with the Anthropic GitHub-app connected; 0 on
//!                    orgs without it (including the dev org).
//!   `prs_total`    — total PRs in the window (`prs_total_count`).
//!                    Denominator for a future `prs_with_cc_pct` ratio.
//!
//! Changes to each `query_ref` (Team + IC, both updated here):
//!   1. Three new `sumIf` expressions in the wide-aggregate (`pp`).
//!   2. Three new `('key', key_v)` entries in the `ARRAY JOIN` unpivot.
//!
//! All three are plain raw counters (not active-markers, not composite
//! ratios). They are aggregated with `avg(v_period)` in the outer
//! dispatch — average per-person value over the requested period —
//! identical to `cc_sessions`, `cc_lines`, etc.
//!
//! Backend-computed `metric_key`s after this migration: 16 → 19.
//! ⚠️  FE-visible status: the three new keys are **not yet rendered** by
//! the frontend — `BULLET_DEFS` / `IC_KPI_DEFS` in `insight-front`
//! need corresponding entries (tracked as a follow-up to INSIGHT-458).
//! Catalog metadata (label / unit / thresholds) is seeded by the paired
//! migration `m20260601_000002_seed_claude_team_metrics_catalog`.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const TEAM_BULLET_AI_ID: &str = "00000000000000000001000000000006";
const IC_BULLET_AI_ID: &str = "00000000000000000001000000000013";

/// Active-counter `metric_key`s — outer uses `sum(v_period)` (count of
/// active persons). Unchanged from `m20260519_000001_ai_bullet_rewrite`.
/// The three new Claude Team metrics are plain counters, NOT active-markers.
const ACTIVE_LIST: &str = "'active_ai_members', 'cursor_active', 'cc_active', 'codex_active'";

/// Inner wide-aggregate: one row per `person_id`, every FE-visible
/// `metric_key` in its own column. Extended with three new `sumIf`s for
/// `cc_cost_v`, `prs_with_cc_v`, `prs_total_v`.
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
         sumIf(metric_value, metric_key = 'prs_with_cc') AS prs_with_cc_v, \
         sumIf(metric_value, metric_key = 'prs_total') AS prs_total_v, \
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
         CAST(NULL AS Nullable(Float64)) AS codex_active_v, \
         CAST(NULL AS Nullable(Float64)) AS chatgpt_v, \
         CAST(NULL AS Nullable(Float64)) AS claude_web_v \
     FROM insight.ai_bullet_rows \
     GROUP BY person_id"
}

/// `ARRAY JOIN` unpivot: 19 wide columns → 19 long rows per person.
/// 16 keys from `m20260519` + 3 new Claude Team keys = 19 total.
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
         ('claude_web',         claude_web_v) \
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

    /// Irreversible — see `m20260519_000001_ai_bullet_rewrite::down` for
    /// the rationale. Rolling back requires reverting the paired CH
    /// migration `20260601000000_ai-claude-team-metrics.sql` first (which
    /// removes `cc_cost`, `prs_with_cc`, `prs_total` from the view), then
    /// restoring the previous `query_ref` manually.
    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom(
            "m20260601_000001_ai_claude_team_metrics is irreversible: \
             roll back the paired CH migration \
             20260601000000_ai-claude-team-metrics.sql first."
                .to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// All 19 FE-visible `metric_key`s must appear in the ARRAY JOIN unpivot.
    /// 16 from m20260519 + 3 new Claude Team keys.
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
    ];

    /// Raw `metric_key`s the wide-aggregate reads from the view via
    /// `sumIf` / `countIf`. A typo here = silent NULL aggregation.
    const EXPECTED_RAW_KEYS_READ_BY_QUERY: &[&str] = &[
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
        "cursor_offered",
        "cc_offered",
        "cursor_total_lines",
    ];

    /// Dropped `metric_keys` from m20260519 that must NOT be read via sumIf.
    const FORBIDDEN_RAW_KEY_READS: &[&str] = &[
        "cursor_acceptance",
        "cc_tool_acceptance",
        "ai_loc_share2",
        "codex_active",
        "chatgpt",
        "claude_web",
    ];

    /// New Claude Team `metric_keys` must NOT appear in `ACTIVE_LIST`
    /// (they are counters, not active-person markers).
    const CLAUDE_TEAM_KEYS: &[&str] = &["cc_cost", "prs_with_cc", "prs_total"];

    fn assert_query_shape(query: &str, label: &str) {
        // Both JOIN sides read from the same source table.
        let table_refs = query.matches("insight.ai_bullet_rows").count();
        assert_eq!(
            table_refs, 2,
            "{label}: expected 2 references to `insight.ai_bullet_rows`, got {table_refs}"
        );

        // Each side has its own GROUP BY person_id wide-aggregate.
        let person_groupbys = query.matches("GROUP BY person_id").count();
        assert_eq!(
            person_groupbys, 2,
            "{label}: expected 2 occurrences of `GROUP BY person_id`, got {person_groupbys}"
        );

        // All 19 FE-visible metric_keys must appear in the ARRAY JOIN.
        for key in EXPECTED_METRIC_KEYS {
            let literal = format!("'{key}'");
            assert!(
                query.contains(&literal),
                "{label}: missing FE-visible metric_key literal {literal} in ARRAY JOIN unpivot"
            );
        }

        // Raw metric_keys read via sumIf / countIf must all be present.
        for key in EXPECTED_RAW_KEYS_READ_BY_QUERY {
            let read = format!("metric_key = '{key}'");
            assert!(
                query.contains(&read),
                "{label}: missing read of raw metric_key `{key}` in wide-aggregate"
            );
        }

        // Dropped metric_keys must NOT be read via sumIf.
        for key in FORBIDDEN_RAW_KEY_READS {
            let read = format!("metric_key = '{key}'");
            assert!(
                !query.contains(&read),
                "{label}: dropped metric_key `{key}` must not be read from the view"
            );
        }

        // New Claude Team keys must NOT be in the ACTIVE_LIST.
        for key in CLAUDE_TEAM_KEYS {
            assert!(
                !ACTIVE_LIST.contains(key),
                "{label}: `{key}` must not be in ACTIVE_LIST — it is a counter, not an active-marker"
            );
        }

        // Active markers must use countIf → 1 else NULL (not sumIf).
        for key in ["active_ai_members", "cursor_active", "cc_active"] {
            let countif_pattern = format!(
                "if(countIf(metric_key = '{key}') > 0, toFloat64(1), CAST(NULL AS Nullable(Float64))) AS {key}_v"
            );
            assert!(
                query.contains(&countif_pattern),
                "{label}: active marker `{key}_v` must use `countIf(...) > 0 → 1 else NULL`"
            );
        }

        // ComingSoon keys must be hardcoded NULL.
        for key in ["codex_active_v", "chatgpt_v", "claude_web_v"] {
            assert!(
                query.contains(&format!("CAST(NULL AS Nullable(Float64)) AS {key}")),
                "{label}: ComingSoon key `{key}` must be hardcoded NULL"
            );
        }

        // Active-counter outer dispatch must be preserved.
        assert!(
            query.contains("p.metric_key IN ('active_ai_members'"),
            "{label}: active-counter outer dispatch must include 'active_ai_members'"
        );
    }

    #[test]
    fn team_query_shape() {
        let q = team_query();
        assert_query_shape(&q, "team_query");
        assert!(
            q.contains("company_median") && q.contains("company_min") && q.contains("company_max"),
            "team_query must expose company_* range"
        );
        assert!(
            !q.contains("team_median"),
            "team_query must NOT use team_median"
        );
        assert!(
            q.contains("ON c.metric_key = p.metric_key"),
            "team_query JOIN must be on metric_key alone"
        );
    }

    #[test]
    fn ic_query_shape() {
        let q = ic_query();
        assert_query_shape(&q, "ic_query");
        assert!(
            q.contains("team_median") && q.contains("team_min") && q.contains("team_max"),
            "ic_query must expose team_* range"
        );
        assert!(
            !q.contains("company_median"),
            "ic_query must NOT use company_median"
        );
        assert!(
            q.contains("c.org_unit_id = p.org_unit_id"),
            "ic_query JOIN must include org_unit_id"
        );
    }

    #[test]
    fn claude_team_keys_are_summed_not_counted() {
        for query in [team_query(), ic_query()] {
            for key in CLAUDE_TEAM_KEYS {
                let sumif = format!("sumIf(metric_value, metric_key = '{key}')");
                assert!(
                    query.contains(&sumif),
                    "Claude Team key `{key}` must be read via sumIf in the wide-aggregate"
                );
                // Must NOT be read via countIf (that's only for active markers).
                let countif = format!("countIf(metric_key = '{key}')");
                assert!(
                    !query.contains(&countif),
                    "Claude Team key `{key}` must NOT be read via countIf — it is a counter"
                );
            }
        }
    }

    #[test]
    fn metric_key_count_is_nineteen() {
        assert_eq!(
            EXPECTED_METRIC_KEYS.len(),
            19,
            "expected 19 FE-visible metric_keys (16 from m20260519 + 3 Claude Team)"
        );
    }
}
