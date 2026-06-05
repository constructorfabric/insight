//! Add cohort distribution (p25/p75 + size `n`) to the Team / IC Bullet
//! AI `query_ref`s. The `c`/`inner_c` cohort aggregation gains
//! `quantileExact(0.25)` / `quantileExact(0.75)` / `count`, surfaced as
//! `p25`/`p75`/`n`. Team cohort stays company-wide; IC keeps the person's
//! department (`org_unit_id`) cohort.
//!
//! These extend the SAME active-counter `multiIf` wrapper used by
//! `median`/`min`/`max`: for the active counters (`active_ai_members`,
//! `cursor_active`, `cc_active`, `codex_active`) the quartiles are NULL
//! (no real distribution — the bullet renders "X out of N active") and
//! `n` mirrors the active `*_max` semantics (`count()` = number of active
//! persons, NULL when the cohort is empty).
//!
//! Team value scoping is done by the handler's `person_id IN (roster)`
//! filter, so the team query keeps the original `GROUP BY metric_key`
//! shape — no supervisor join. Both bullet-row leaves still
//! `GROUP BY person_id`, so `inject_date_filter_into_subqueries` injects
//! the metric_date range before each GROUP BY exactly as before.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const TEAM_BULLET_AI_ID: &str = "00000000000000000001000000000006";
const IC_BULLET_AI_ID: &str = "00000000000000000001000000000013";

/// Active-counter `metric_key`s — outer uses `sum(v_period)` (count of
/// active persons), and `range_*` / `p25` / `p75` / `n` use the
/// hardcoded "X out of N" semantics rather than a real quantile
/// distribution.
const ACTIVE_LIST: &str = "'active_ai_members', 'cursor_active', 'cc_active', 'codex_active'";

/// Inner wide-aggregate block: one row per `person_id` with every
/// FE-visible `metric_key` materialized in its own column. Copied
/// verbatim from `m20260519_000001_ai_bullet_rewrite::wide_aggregate_pp`.
/// Used by both sides of both queries.
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

/// `ARRAY JOIN` unpivot: 16 wide columns → 16 long rows per person.
/// Copied verbatim from `m20260519_000001_ai_bullet_rewrite`.
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
                any(c.company_max) AS range_max, \
                any(c.company_p25) AS p25, \
                any(c.company_p75) AS p75, \
                any(c.company_n) AS n \
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
                            max(v_period)) AS company_max, \
                    multiIf(metric_key IN ({ACTIVE_LIST}), \
                            CAST(NULL AS Nullable(Float64)), \
                            quantileExact(0.25)(v_period)) AS company_p25, \
                    multiIf(metric_key IN ({ACTIVE_LIST}), \
                            CAST(NULL AS Nullable(Float64)), \
                            quantileExact(0.75)(v_period)) AS company_p75, \
                    multiIf(metric_key IN ({ACTIVE_LIST}), \
                            if(count(v_period) = 0, CAST(NULL AS Nullable(Float64)), toFloat64(count())), \
                            toFloat64(count(v_period))) AS company_n \
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
                    multiIf(metric_key IN ({ACTIVE_LIST}), \
                            if(count(v_period) = 0, CAST(NULL AS Nullable(Float64)), toFloat64(0)), \
                            quantileExact(0.5)(v_period)) AS team_median, \
                    multiIf(metric_key IN ({ACTIVE_LIST}), \
                            if(count(v_period) = 0, CAST(NULL AS Nullable(Float64)), toFloat64(0)), \
                            min(v_period)) AS team_min, \
                    multiIf(metric_key IN ({ACTIVE_LIST}), \
                            if(count(v_period) = 0, CAST(NULL AS Nullable(Float64)), toFloat64(count())), \
                            max(v_period)) AS team_max, \
                    multiIf(metric_key IN ({ACTIVE_LIST}), \
                            CAST(NULL AS Nullable(Float64)), \
                            quantileExact(0.25)(v_period)) AS team_p25, \
                    multiIf(metric_key IN ({ACTIVE_LIST}), \
                            CAST(NULL AS Nullable(Float64)), \
                            quantileExact(0.75)(v_period)) AS team_p75, \
                    multiIf(metric_key IN ({ACTIVE_LIST}), \
                            if(count(v_period) = 0, CAST(NULL AS Nullable(Float64)), toFloat64(count())), \
                            toFloat64(count(v_period))) AS team_n \
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

/// Predecessor (`m20260519_000001_ai_bullet_rewrite`) team `query_ref`,
/// restored verbatim by `down()`.
fn old_team_query() -> String {
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

/// Predecessor (`m20260519_000001_ai_bullet_rewrite`) IC `query_ref`,
/// restored verbatim by `down()`.
fn old_ic_query() -> String {
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

    /// Restore the predecessor `m20260519_000001_ai_bullet_rewrite`
    /// `query_ref`s (company-wide team cohort, no `p25` / `p75` / `n`).
    /// This down is safe in isolation: it only narrows the response shape
    /// back to the predecessor's columns and re-points the team cohort at
    /// the company-wide grouping — no view dependency, unlike the
    /// predecessor's own irreversible `down()`.
    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        for (hex_id, query) in [
            (TEAM_BULLET_AI_ID, old_team_query()),
            (IC_BULLET_AI_ID, old_ic_query()),
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
    // `m20260519_000001_ai_bullet_rewrite::tests`. Goal: catch the
    // high-impact regressions a typo in this PR would cause (silent NULL
    // aggregation from a misspelled `metric_key`, missing composite-ratio
    // formula, `ComingSoon` hardcode drift, active-counter dispatch
    // broken, and — new here — missing quartile columns).

    /// Every FE-visible `metric_key` the bullet section emits must appear
    /// as an `('X', X_v)` entry in the ARRAY JOIN unpivot.
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
        "cursor_acceptance",
        "cc_tool_acceptance",
        "ai_loc_share2",
        "codex_active",
        "chatgpt",
        "claude_web",
    ];

    /// Every raw `metric_key` the view emits that `query_ref` reads via
    /// `sumIf` / `countIf` must appear as a literal. A typo = silent NULL.
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
        "cursor_offered",
        "cc_offered",
        "cursor_total_lines",
    ];

    /// `metric_key`s that must NOT be read via `sumIf` — the view no
    /// longer emits them, so a `metric_key = 'X'` read would silently
    /// aggregate to NULL.
    const FORBIDDEN_RAW_KEY_READS: &[&str] = &[
        "cursor_acceptance",
        "cc_tool_acceptance",
        "ai_loc_share2",
        "codex_active",
        "chatgpt",
        "claude_web",
    ];

    fn assert_query_shape(query: &str, label: &str) {
        // Both sides of the JOIN read from the same source table.
        let table_refs = query.matches("insight.ai_bullet_rows").count();
        assert_eq!(
            table_refs, 2,
            "{label}: expected 2 references to `insight.ai_bullet_rows` (one per JOIN side), got {table_refs}"
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
        // match what the view emits.
        for key in EXPECTED_RAW_KEYS_READ_BY_QUERY {
            let read = format!("metric_key = '{key}'");
            assert!(
                query.contains(&read),
                "{label}: missing read of raw metric_key {key} (`metric_key = '{key}'`) in wide-aggregate"
            );
        }

        // The dropped metric_keys must NOT be read via sumIf/countIf.
        for key in FORBIDDEN_RAW_KEY_READS {
            let read = format!("metric_key = '{key}'");
            assert!(
                !query.contains(&read),
                "{label}: dropped metric_key {key} must not be read from the view (it's no longer emitted)"
            );
        }

        // `ComingSoon` keys must be hardcoded NULL columns in the
        // wide-aggregate (the honest-NULL contract).
        for key in ["codex_active_v", "chatgpt_v", "claude_web_v"] {
            assert!(
                query.contains(&format!("CAST(NULL AS Nullable(Float64)) AS {key}")),
                "{label}: `ComingSoon` key alias {key} must be hardcoded NULL via `CAST(NULL AS Nullable(Float64)) AS {key}`"
            );
        }

        // Active-counter outer dispatch must be preserved — without it,
        // active_* metrics would be averaged instead of summed.
        assert!(
            query.contains("p.metric_key IN ('active_ai_members'"),
            "{label}: active-counter outer dispatch must include 'active_ai_members' in the IN-list"
        );
        for key in ["cursor_active", "cc_active", "codex_active"] {
            assert!(
                query.contains(&format!("'{key}'")),
                "{label}: active-counter list must include '{key}'"
            );
        }

        // Active markers must collapse via `countIf(...) > 0 → 1, else NULL`.
        for key in ["active_ai_members", "cursor_active", "cc_active"] {
            let countif_pattern = format!(
                "if(countIf(metric_key = '{key}') > 0, toFloat64(1), CAST(NULL AS Nullable(Float64))) AS {key}_v"
            );
            assert!(
                query.contains(&countif_pattern),
                "{label}: active marker `{key}_v` must use `countIf(...) > 0 → 1 else NULL` (not sumIf)"
            );
        }

        // The active-counter `multiIf` wrapper must also gate the new
        // distribution columns (p25/p75/n) — without it the quartiles
        // would compute a meaningless distribution over the {1, NULL}
        // active marker instead of NULL.
        assert!(
            query.contains("multiIf(metric_key IN ('active_ai_members'"),
            "{label}: distribution columns must extend the active-counter multiIf wrapper (query must contain `multiIf(metric_key IN ('active_ai_members'`)"
        );
    }

    #[test]
    fn team_query_shape() {
        let q = team_query();
        assert_query_shape(&q, "team_query");

        // Team-scope: company-wide cohort labels (not partitioned by
        // org_unit_id).
        assert!(
            q.contains("company_median") && q.contains("company_min") && q.contains("company_max"),
            "team_query must expose company_* range, got:\n{q}"
        );
        assert!(
            !q.contains("team_median"),
            "team_query must NOT use team_median (that's the IC-side label)"
        );
        assert!(
            q.contains("ON c.metric_key = p.metric_key"),
            "team_query JOIN must be on metric_key alone"
        );

        // New cohort distribution columns surfaced on the outer SELECT.
        for col in ["company_p25", "company_p75", "company_n"] {
            assert!(
                q.contains(col),
                "team_query must expose cohort column {col}, got:\n{q}"
            );
        }
        assert!(
            q.contains("any(c.company_p25) AS p25")
                && q.contains("any(c.company_p75) AS p75")
                && q.contains("any(c.company_n) AS n"),
            "team_query outer SELECT must alias p25/p75/n from the company cohort, got:\n{q}"
        );
        assert!(
            q.contains("quantileExact(0.25)(v_period)")
                && q.contains("quantileExact(0.75)(v_period)"),
            "team_query cohort must compute quartiles via quantileExact(0.25)/(0.75), got:\n{q}"
        );

        // Roster scoping happens at the handler (`person_id IN (...)`), so
        // the query stays company-wide + groups by metric_key only — no
        // supervisor join, no supervisor_email column.
        assert!(
            q.contains("GROUP BY p.metric_key"),
            "team_query outer GROUP BY must be metric_key, got:\n{q}"
        );
        assert!(
            !q.contains("supervisor_email"),
            "team_query must NOT reference supervisor_email (roster scope is at the handler)"
        );
        assert!(
            !q.contains("insight.people"),
            "team_query must NOT join insight.people"
        );

        // Both leaves still GROUP BY person_id (p and inner_c).
        let person_groupbys = q.matches("GROUP BY person_id").count();
        assert_eq!(
            person_groupbys, 2,
            "team_query: expected 2 occurrences of `GROUP BY person_id` (p and inner_c), got {person_groupbys}"
        );
    }

    #[test]
    fn ic_query_shape() {
        let q = ic_query();
        assert_query_shape(&q, "ic_query");

        // IC-scope: team-wide cohort labels (partitioned by org_unit_id).
        assert!(
            q.contains("team_median") && q.contains("team_min") && q.contains("team_max"),
            "ic_query must expose team_* range, got:\n{q}"
        );
        assert!(
            !q.contains("company_median"),
            "ic_query must NOT use company_median (that's the Team-side label)"
        );
        assert!(
            q.contains("c.org_unit_id = p.org_unit_id"),
            "ic_query JOIN must include org_unit_id"
        );

        // New cohort distribution columns surfaced on the outer SELECT.
        for col in ["team_p25", "team_p75", "team_n"] {
            assert!(
                q.contains(col),
                "ic_query must expose cohort column {col}, got:\n{q}"
            );
        }
        assert!(
            q.contains("any(c.team_p25) AS p25")
                && q.contains("any(c.team_p75) AS p75")
                && q.contains("any(c.team_n) AS n"),
            "ic_query outer SELECT must alias p25/p75/n from the team cohort, got:\n{q}"
        );
        assert!(
            q.contains("quantileExact(0.25)(v_period)")
                && q.contains("quantileExact(0.75)(v_period)"),
            "ic_query cohort must compute quartiles via quantileExact(0.25)/(0.75), got:\n{q}"
        );

        // IC variant is unchanged w.r.t. supervisor scope: no people-join,
        // no supervisor_email, and both leaves still GROUP BY person_id.
        assert!(
            !q.contains("supervisor_email"),
            "ic_query must NOT carry supervisor_email (that's the team-only rescope), got:\n{q}"
        );
        assert!(
            !q.contains("LEFT JOIN insight.people"),
            "ic_query must NOT join insight.people, got:\n{q}"
        );
        let person_groupbys = q.matches("GROUP BY person_id").count();
        assert_eq!(
            person_groupbys, 2,
            "ic_query: expected 2 occurrences of `GROUP BY person_id` (p and inner_c, unchanged), got {person_groupbys}"
        );
    }

    /// `down()` must restore the predecessor shape exactly: company-wide
    /// team cohort, no quartile columns, no supervisor scope.
    #[test]
    fn down_restores_predecessor_shape() {
        let team = old_team_query();
        let ic = old_ic_query();

        // No new distribution columns.
        for q in [&team, &ic] {
            assert!(
                !q.contains("quantileExact(0.25)") && !q.contains("quantileExact(0.75)"),
                "down() query must not carry the new quartile columns, got:\n{q}"
            );
            assert!(
                !q.contains(") AS p25") && !q.contains(") AS p75") && !q.contains(") AS n"),
                "down() query must not expose p25/p75/n, got:\n{q}"
            );
        }

        // No supervisor rescope in the restored team query.
        assert!(
            !team.contains("supervisor_email") && !team.contains("LEFT JOIN insight.people"),
            "down() team query must be company-wide (no supervisor scope), got:\n{team}"
        );
        assert!(
            team.contains("GROUP BY p.metric_key") && !team.contains("p.supervisor_email"),
            "down() team query must group by metric_key alone, got:\n{team}"
        );

        // Predecessor cohort labels intact.
        assert!(
            team.contains("company_median") && ic.contains("team_median"),
            "down() must restore predecessor cohort labels"
        );
    }
}
