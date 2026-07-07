//! Rewrite the Team / IC Bullet AI `query_ref`s to consume the new
//! `ai_bullet_rows` shape (issue #433 §4.1, §4.4).
//!
//! Pairs with ingestion migration `20260519000000_ai-bullet-rewrite.sql`,
//! which drops the daily % from the view for 3 ratio metrics, drops the
//! 3 `ComingSoon` NULL-only emissions, and emits raw num/den counters
//! instead. The `query_ref`s now reconstruct the composites as
//! `100 * Σnum / Σden` over the period — the only mathematically
//! correct period aggregation when daily denominators differ (CLAUDE.md
//! "Aggregation correctness").
//!
//! Mathematical changes:
//!   - `cursor_acceptance`  avg(daily 100*accepted/offered)
//!     → 100 * `Σcursor_completions` / `Σcursor_offered`
//!     (`cursor_completions` IS the per-day accepted count, aliased per #262)
//!   - `cc_tool_acceptance` avg(daily 100*accepted/offered)
//!     → 100 * `Σcc_tool_accept` / `Σcc_offered`
//!   - `ai_loc_share2`      avg(daily 100*`lines_added`/`total_lines_added`)
//!     → 100 * `Σcursor_lines` / `Σcursor_total_lines`
//!
//! Bug-fix carried in this PR:
//!   - `cc_tool_accept` was falling through to the `avg(metric_value)`
//!     default in the predecessor `multiIf` dispatch (it is not in the
//!     `SUM_LIST` and not in the `COUNT_LIST`). It is a raw counter
//!     (`coalesce(tool_use_accepted, 0)` per day) and should be summed
//!     over the period, not averaged. The new wide-aggregate computes
//!     it via `sumIf` like every other raw counter.
//!
//! `ComingSoon` audit (issue #433 §4.4):
//!   - `codex_active`, `chatgpt`, `claude_web` are not ingested. The
//!     predecessor view emitted one NULL-valued row per (person, date)
//!     from `silver.class_ai_dev_usage` for each of these — pure noise
//!     that inflated the view. The paired CH migration drops those
//!     branches entirely. The corresponding FE-visible `metric_key`s
//!     are preserved in the response shape because this `query_ref`
//!     hardcodes them to NULL columns in the wide-aggregate — the
//!     honest-NULL → `ComingSoon` contract from
//!     `20260423120000_bullet-views-honest-nulls.sql` ("flip those to
//!     NULL so the FE bullet renders `ComingSoon`").
//!
//! Active-counter outer dispatch preserved unchanged:
//!   `active_ai_members`, `cursor_active`, `cc_active`, `codex_active`
//!   keep the predecessor's `sum(p.v_period)` outer aggregation — they
//!   are person-counts, not per-person quantities to be averaged
//!   ("how many distinct persons were active in the period").
//!   Likewise the company / team range for these is the same special
//!   case as before: median = 0, min = 0, max = count of persons with
//!   a non-NULL `v_period`. This is intentionally NOT a real
//!   distribution — the bullet renders "X out of N team members
//!   active".
//!
//! Structural change (mirrors PR #478 / #480):
//!   - Replaced `multiIf(metric_key=X, dispatch)` inner with
//!     wide-aggregate per `metric_key` + `ARRAY JOIN` unpivot back to
//!     long format. Outer keeps the `multiIf(active vs non-active,
//!     sum vs avg)` switch because active counters and value metrics
//!     genuinely need different outer aggregation semantics.
//!
//! Walker compatibility: each query has exactly two leaf subqueries
//! that read from `insight.ai_bullet_rows GROUP BY person_id` (one in
//! `p`, one in `inner_c`). `inject_date_filter_into_subqueries` in
//! `handlers.rs` walks both and injects
//! `WHERE metric_date >= … AND <` before the `GROUP BY` in each leaf —
//! same behavior as the predecessor.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const TEAM_BULLET_AI_ID: &str = "00000000000000000001000000000006";
const IC_BULLET_AI_ID: &str = "00000000000000000001000000000013";

/// Active-counter `metric_key`s — outer uses `sum(v_period)` (count of
/// active persons), and `range_*` use the hardcoded `0 / 0 / count()`
/// "X out of N" semantics rather than a real quantile distribution.
const ACTIVE_LIST: &str = "'active_ai_members', 'cursor_active', 'cc_active', 'codex_active'";

/// Inner wide-aggregate block: one row per `person_id` with every
/// FE-visible `metric_key` materialized in its own column.
///   - Active markers (`*_active_v`): `1` if the person had any row for
///     that tool in the period, NULL otherwise — so the outer `sum()`
///     counts active persons and `count(v_period)` in the c side
///     excludes inactives.
///   - Raw counters: `sumIf` over the period.
///   - Composite ratios: `100 * Σnum / Σden` with NULL on zero
///     denominator, so the outer `avg()` ignores undefined cases.
///   - `ComingSoon` (`codex_active`, `chatgpt`, `claude_web`): hardcoded
///     NULL — the view no longer emits these, and the honest-NULL
///     contract from `20260423120000_bullet-views-honest-nulls.sql`
///     renders them as `ComingSoon` on the FE.
///
/// `pp` is the output alias used by the caller.
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
/// The 13 view-emitted keys + 3 `ComingSoon` hardcoded-NULL keys =
/// 16 FE-visible `metric_key`s (matches the predecessor's response set).
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

    /// Explicitly irreversible. The paired CH migration
    /// `20260519000000_ai-bullet-rewrite.sql` redefines
    /// `insight.ai_bullet_rows` to drop 3 composite-ratio `metric_key`s
    /// (`cursor_acceptance`, `cc_tool_acceptance`, `ai_loc_share2`) and
    /// 3 `ComingSoon` NULL-only emissions (`codex_active`, `chatgpt`,
    /// `claude_web`). Restoring the old `query_ref` here without first
    /// reverting the view would leave the queries pointing at
    /// `metric_key`s the view no longer emits — the bullets would
    /// silently render ``ComingSoon`` for the 3 ratios (incorrectly) and
    /// fall through `multiIf` defaults for the 3 audit drops. Roll back
    /// by reverting the paired CH migration first, then this `down()`.
    /// Same pattern as `m20260428_000001_collab_metrics_update` and
    /// `m20260518_000001_collab_bullet_rewrite`.
    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom(
            "m20260519_000001_ai_bullet_rewrite is irreversible: \
             roll back the paired CH migration 20260519000000_ai-bullet-rewrite.sql \
             (which drops the 3 composite metric_keys and the 3 `ComingSoon` \
             NULL-only emissions from the view) before reverting metrics.query_ref."
                .to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // String-contains tests — same rationale as
    // `m20260515_000001_task_delivery_bullet_rewrite::tests` and
    // `m20260518_000001_collab_bullet_rewrite::tests`. Goal: catch the
    // high-impact regressions a typo in this PR would cause (silent
    // NULL aggregation from a misspelled `metric_key`, missing
    // composite-ratio formula, `ComingSoon` hardcode drift,
    // active-counter outer dispatch broken).

    /// Every FE-visible `metric_key` the bullet section emits must
    /// appear as an `('X', X_v)` entry in the ARRAY JOIN unpivot.
    /// 13 view-emitted + 3 `ComingSoon` hardcoded = 16 total.
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
    /// `sumIf` / `countIf` must appear as a literal. A typo here =
    /// silent NULL aggregation.
    /// 7 raw sum counters + 3 active markers + 3 denominator counters
    /// = 13 view-emitted keys (= the rewritten view's `metric_key` set).
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
        // Dropped composites (now computed from raw num/den):
        "cursor_acceptance",
        "cc_tool_acceptance",
        "ai_loc_share2",
        // Dropped `ComingSoon` audit emissions (now hardcoded NULL):
        "codex_active",
        "chatgpt",
        "claude_web",
    ];

    fn assert_query_shape(query: &str, label: &str) {
        // Both sides of the JOIN read from the same source table.
        let table_refs = query.matches("insight.ai_bullet_rows").count();
        assert_eq!(
            table_refs, 2,
            "{label}: expected 2 references to `insight.ai_bullet_rows` (one per JOIN side, no CTE hoist yet — see issue #433 §3.4), got {table_refs}"
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
        // match what the view emits.
        for key in EXPECTED_RAW_KEYS_READ_BY_QUERY {
            let read = format!("metric_key = '{key}'");
            assert!(
                query.contains(&read),
                "{label}: missing read of raw metric_key {key} (`metric_key = '{key}'`) in wide-aggregate"
            );
        }

        // The dropped metric_keys must NOT be read via sumIf/countIf —
        // the view no longer emits them.
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
        // active_* metrics would be averaged instead of summed (= avg
        // of {0,1} ≈ active_fraction instead of active_count).
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

        // Active markers must collapse via `countIf(...) > 0 → 1, else NULL`
        // — NOT `sumIf(metric_value, ...)`. Branch 1 of the view emits
        // ('active_ai_members', 1) once per (person, date, tool) row, so a
        // user with both Cursor and Claude on the same day produces two
        // rows. A `sumIf` would inflate the per-person marker to 2 (and
        // outer sum across persons would double-count). `countIf > 0 → 1`
        // collapses correctly to a single 1 regardless of row count.
        for key in ["active_ai_members", "cursor_active", "cc_active"] {
            let countif_pattern = format!(
                "if(countIf(metric_key = '{key}') > 0, toFloat64(1), CAST(NULL AS Nullable(Float64))) AS {key}_v"
            );
            assert!(
                query.contains(&countif_pattern),
                "{label}: active marker `{key}_v` must use `countIf(...) > 0 → 1 else NULL` (not sumIf — Branch 1 emits multiple rows per person-day for multi-tool users)"
            );
        }
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
        assert!(
            q.contains("c.org_unit_id = p.org_unit_id"),
            "ic_query JOIN must include org_unit_id"
        );
    }
}
