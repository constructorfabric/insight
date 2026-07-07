//! Reconcile the Team / IC Bullet AI `query_ref`s after the merge of two
//! independent lines of work on the same two metrics (`…0006` Team, `…0013`
//! IC):
//!
//!   - `m20260609_000001` un-stubbed ChatGPT/Codex and grew the AI bullet to
//!     22 `metric_key`s (real `codex_active`/`chatgpt`, new
//!     `cc_cost`/`prs_with_cc`/`prs_total`/`codex_lines`/`codex_sessions`/
//!     `chatgpt_active`, with `chatgpt_active` added to `ACTIVE_LIST`) — but
//!     it carries no cohort distribution and the Team side stays company-wide.
//!   - `m20260604_000003` added the `p25`/`p75`/`n` cohort distribution to
//!     both AI bullets, and `m20260606_000003` rebased the Team bullet onto a
//!     headcount-weighted blend of its members' DEPARTMENT cohorts
//!     (`any(c.*)` → `avg(c.*)`, joined on `org_unit_id`).
//!
//! Whichever of those ran last would clobber the other's AI work, and the
//! winner would differ between a fresh database and one that already applied
//! `…0609`. This migration is registered last and re-seeds both AI bullets to
//! the intended union, so the final state is deterministic on any history:
//!
//!   - shared wide-aggregate / ARRAY JOIN / `ACTIVE_LIST` = the 22-key set
//!     from `m20260609_000001` (copied verbatim);
//!   - IC (`…0013`): the department cohort with the `p25`/`p75`/`n`
//!     distribution (the `m20260604_000003` IC shape), `any(c.team_*)`;
//!   - Team (`…0006`): the same department cohort, blended with `avg(c.team_*)`
//!     and `n = count(p.v_period)` (the `m20260606_000003` Team shape).
//!
//! The active-marker keys keep their `multiIf` null-guard (`p25`/`p75` NULL, the
//! "X of N active" semantics for median/min/max), so `avg(NULL)` stays NULL
//! and they render neutral — correct for member-scale counters.
//!
//! `down()` restores the `m20260609_000001` `query_ref`s verbatim (the
//! immediately-prior committed state for these two metrics).

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const TEAM_BULLET_AI_ID: &str = "00000000000000000001000000000006";
const IC_BULLET_AI_ID: &str = "00000000000000000001000000000013";

/// Active-marker `metric_key`s — outer aggregation is `sum(v_period)` (count
/// of active persons); their cohort `p25`/`p75` are NULL. Copied verbatim
/// from `m20260609_000001` (includes `chatgpt_active`).
const ACTIVE_LIST: &str =
    "'active_ai_members', 'cursor_active', 'cc_active', 'codex_active', 'chatgpt_active'";

/// 22-key wide-aggregate, copied verbatim from `m20260609_000001`.
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
         if(countIf(metric_key = 'chatgpt_active') > 0, toFloat64(1), CAST(NULL AS Nullable(Float64))) AS chatgpt_active_v \
     FROM insight.ai_bullet_rows \
     GROUP BY person_id"
}

/// 22-key ARRAY JOIN unpivot, copied verbatim from `m20260609_000001`.
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
         ('chatgpt_active',     chatgpt_active_v) \
     ] AS kv"
}

/// Department cohort with the `p25`/`p75`/`n` distribution and the
/// active-marker `multiIf` null-guard (shared by the new IC + Team queries).
/// Joined on `(metric_key, org_unit_id)`.
fn dept_cohort_join(pp: &str, kv: &str) -> String {
    format!(
        "LEFT JOIN ( \
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
         ) c ON c.metric_key = p.metric_key AND c.org_unit_id = p.org_unit_id"
    )
}

/// NEW IC AI query (`…0013`): 22-key aggregate + department cohort with the
/// distribution surfaced via `any(c.team_*)`.
fn new_ic_query() -> String {
    let pp = wide_aggregate_pp();
    let kv = array_join_kv();
    let cohort = dept_cohort_join(pp, kv);
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
         {cohort} \
         GROUP BY p.metric_key"
    )
}

/// NEW Team AI query (`…0006`): 22-key aggregate + department cohort blended
/// per roster member via `avg(c.team_*)`; `n = count(p.v_period)`.
fn new_team_query() -> String {
    let pp = wide_aggregate_pp();
    let kv = array_join_kv();
    let cohort = dept_cohort_join(pp, kv);
    format!(
        "SELECT p.metric_key AS metric_key, \
                multiIf(p.metric_key IN ({ACTIVE_LIST}), sum(p.v_period), avg(p.v_period)) AS value, \
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
         {cohort} \
         GROUP BY p.metric_key"
    )
}

/// `m20260609_000001` Team `query_ref` (company cohort, median/min/max only),
/// restored by `down()`.
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

/// `m20260609_000001` IC `query_ref` (department cohort, median/min/max only),
/// restored by `down()`.
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
            (TEAM_BULLET_AI_ID, new_team_query()),
            (IC_BULLET_AI_ID, new_ic_query()),
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

    /// All 22 keys from `m20260609_000001` must survive the reconcile.
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
    ];

    fn assert_has_22_keys(q: &str, label: &str) {
        for key in EXPECTED_METRIC_KEYS {
            assert!(
                q.contains(&format!("('{key}',")),
                "{label}: missing key {key}"
            );
        }
        assert!(
            q.contains("chatgpt_active") && q.contains("codex_lines"),
            "{label}: must carry the m20260609 ChatGPT/Codex keys"
        );
    }

    #[test]
    fn ic_keeps_department_cohort_with_distribution() {
        let q = new_ic_query();
        assert_has_22_keys(&q, "new_ic_query");
        assert!(
            q.contains("c.org_unit_id = p.org_unit_id"),
            "IC must join the department cohort on org_unit_id"
        );
        for col in [
            "any(c.team_p25) AS p25",
            "any(c.team_p75) AS p75",
            "any(c.team_n) AS n",
        ] {
            assert!(q.contains(col), "IC must surface distribution via `{col}`");
        }
        assert!(
            !q.contains("avg(c.team_"),
            "IC must NOT blend (that's the Team shape)"
        );
    }

    #[test]
    fn team_blends_department_cohort_with_distribution() {
        let q = new_team_query();
        assert_has_22_keys(&q, "new_team_query");
        assert!(
            q.contains("c.org_unit_id = p.org_unit_id"),
            "Team must join the department cohort on org_unit_id"
        );
        for col in ["avg(c.team_median)", "avg(c.team_p25)", "avg(c.team_p75)"] {
            assert!(q.contains(col), "Team must blend the cohort via `{col}`");
        }
        assert!(
            q.contains("toFloat64(count(p.v_period)) AS n"),
            "Team cohort size must be count(p.v_period)"
        );
        assert!(
            !q.contains("any(c.team_p25)"),
            "Team must NOT use any() for the cohort (that's the IC shape)"
        );
        // Value keeps the active-aware multiIf (sum for active, avg otherwise).
        assert!(
            q.contains(&format!(
                "multiIf(p.metric_key IN ({ACTIVE_LIST}), sum(p.v_period), avg(p.v_period)) AS value"
            )),
            "Team value must keep the active-counter multiIf"
        );
    }

    #[test]
    fn active_markers_null_their_quartiles() {
        // chatgpt_active is in ACTIVE_LIST, so the cohort multiIf NULLs p25/p75
        // for the active markers while real keys keep the quantile branch.
        assert!(ACTIVE_LIST.contains("'chatgpt_active'"));
        for q in [new_team_query(), new_ic_query()] {
            assert!(
                q.contains("quantileExact(0.25)(v_period)) AS team_p25")
                    && q.contains("quantileExact(0.75)(v_period)) AS team_p75"),
                "cohort must keep the real quartile branch for non-active keys"
            );
            assert!(
                q.contains(&format!("multiIf(metric_key IN ({ACTIVE_LIST}),")),
                "cohort must gate the active markers via the multiIf wrapper"
            );
        }
    }

    #[test]
    fn down_restores_m20260609_shape() {
        let team = old_team_query();
        let ic = old_ic_query();
        assert_has_22_keys(&team, "old_team_query");
        assert_has_22_keys(&ic, "old_ic_query");
        // Company cohort on the Team side, no distribution columns.
        assert!(
            team.contains("any(c.company_median)")
                && team.contains("ON c.metric_key = p.metric_key")
        );
        for q in [&team, &ic] {
            assert!(
                !q.contains(") AS p25") && !q.contains(") AS p75") && !q.contains(") AS n"),
                "down() must drop the distribution columns (m20260609 shape)"
            );
        }
        assert!(
            ic.contains("c.org_unit_id = p.org_unit_id"),
            "IC down keeps department cohort"
        );
    }
}
