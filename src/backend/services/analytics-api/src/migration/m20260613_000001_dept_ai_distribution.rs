//! Per-(department, metric) distribution for the AI bullet section (`…0048`),
//! completing the `…0044-46` set (Task Delivery / Collaboration / Git) so the
//! team view's AI adoption card can roll up its status from per-member-vs-own-
//! department standings like the other three sections.
//!
//! Shape matches `m20260606_000001`:
//!   `org_unit_id, metric_key, p25, median, p75, range_min, range_max, n`,
//! grouped per `(org_unit_id, metric_key)`. The per-person wide aggregate +
//! ARRAY JOIN are copied from `m20260606_000003`'s AI team query
//! (`insight.ai_bullet_rows`), per the repo convention that a migration
//! captures the exact SQL it installs.
//!
//! Two deliberate omissions from the ARRAY JOIN vs the AI team bullet:
//!   - the active-counter flags (`active_ai_members`, `cursor_active`,
//!     `cc_active`, `codex_active`) — member-scale 0/1 signals with no
//!     meaningful per-person distribution (the IC/team bullets already NULL
//!     their band); leaving them out gives them no department cohort, so the
//!     rollup treats them as neutral.
//!   - the all-NULL placeholders (`chatgpt`, `claude_web`) — no source data.
//!
//! The remaining ratio/volume keys carry real per-person distributions. Values
//! are Nullable (a ratio is NULL when its denominator is zero), so the quartile
//! / range / count aggregators use the `*If(isNotNull(v_period))` family (as in
//! the Git distribution) to skip NULLs rather than fold them in.
//!
//! The per-person leaf keeps `GROUP BY person_id`, so the handler's date-walker
//! injects the `metric_date` range there; the outer `GROUP BY org_unit_id,
//! metric_key` and any `org_unit_id IN (...)` filter are re-applied by the
//! handler. `down()` deletes the metric (append-only seed).

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const ZERO_TENANT: &str = "00000000000000000000000000000000";
const DEPT_DIST_AI_HEX: &str = "00000000000000000001000000000048";

/// Per-person AI wide aggregate, copied verbatim from `m20260606_000003`'s
/// `ai_wide_aggregate_pp` (`insight.ai_bullet_rows`, one row per person). The
/// active-counter / placeholder columns are computed but simply not unpivoted
/// by `ai_array_join_kv` below.
fn ai_wide_aggregate_pp() -> &'static str {
    "SELECT person_id, any(org_unit_id) AS org_unit_id, \
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
            CAST(NULL AS Nullable(Float64))) AS ai_loc_share2_v \
     FROM insight.ai_bullet_rows \
     GROUP BY person_id"
}

/// ARRAY JOIN unpivot for the distributable AI keys only — active-counter flags
/// and the all-NULL placeholders are intentionally excluded (see module doc).
fn ai_array_join_kv() -> &'static str {
    "ARRAY JOIN [ \
         ('cursor_completions', cursor_completions_v), \
         ('cursor_agents',      cursor_agents_v), \
         ('cursor_lines',       cursor_lines_v), \
         ('cc_sessions',        cc_sessions_v), \
         ('cc_lines',           cc_lines_v), \
         ('cc_tool_accept',     cc_tool_accept_v), \
         ('team_ai_loc',        team_ai_loc_v), \
         ('cursor_acceptance',  cursor_acceptance_v), \
         ('cc_tool_acceptance', cc_tool_acceptance_v), \
         ('ai_loc_share2',      ai_loc_share2_v) \
     ] AS kv"
}

fn ai_query() -> String {
    let pp = ai_wide_aggregate_pp();
    let kv = ai_array_join_kv();
    format!(
        "SELECT org_unit_id, metric_key, \
                quantileExactIf(0.25)(v_period, isNotNull(v_period)) AS p25, \
                quantileExactIf(0.5)(v_period, isNotNull(v_period)) AS median, \
                quantileExactIf(0.75)(v_period, isNotNull(v_period)) AS p75, \
                minIf(v_period, isNotNull(v_period)) AS range_min, \
                maxIf(v_period, isNotNull(v_period)) AS range_max, \
                countIf(isNotNull(v_period)) AS n \
         FROM ( \
             SELECT org_unit_id, kv.1 AS metric_key, kv.2 AS v_period \
             FROM ({pp}) ppc \
             {kv} \
         ) inner_c \
         GROUP BY org_unit_id, metric_key"
    )
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        db.execute_unprepared(&format!(
            "INSERT INTO metrics (id, insight_tenant_id, name, description, query_ref, is_enabled) \
             VALUES (UNHEX('{DEPT_DIST_AI_HEX}'), UNHEX('{ZERO_TENANT}'), 'Dept Distribution — AI', \
             'Per-(department, metric) quartile distribution for the distributable AI bullet keys, from insight.ai_bullet_rows (active-counter flags and NULL placeholders excluded). Filter by org_unit_id IN (...).', \
             '{qr}', 1) \
             ON DUPLICATE KEY UPDATE name=VALUES(name), description=VALUES(description), query_ref=VALUES(query_ref), is_enabled=1",
            qr = ai_query().replace('\'', "''"),
        ))
        .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        db.execute_unprepared(&format!(
            "DELETE FROM metrics WHERE id = UNHEX('{DEPT_DIST_AI_HEX}')"
        ))
        .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ai_query_shape() {
        let q = ai_query();
        assert!(
            q.starts_with("SELECT org_unit_id, metric_key,"),
            "outer projection must lead with `org_unit_id, metric_key`, got:\n{q}"
        );
        assert!(
            q.contains("GROUP BY org_unit_id, metric_key"),
            "outer GROUP BY must be `org_unit_id, metric_key`, got:\n{q}"
        );
        for alias in [
            "AS p25",
            "AS median",
            "AS p75",
            "AS range_min",
            "AS range_max",
            "AS n",
        ] {
            assert!(
                q.contains(alias),
                "missing output alias `{alias}`, got:\n{q}"
            );
        }
        assert!(
            q.contains("insight.ai_bullet_rows"),
            "must read insight.ai_bullet_rows, got:\n{q}"
        );
        // Nullable AI ratios → *If(isNotNull) family (as in the Git distribution).
        assert!(
            q.contains("quantileExactIf(0.5)(v_period, isNotNull(v_period)) AS median")
                && q.contains("countIf(isNotNull(v_period)) AS n"),
            "quartile/count aggregators must skip NULLs via *If(isNotNull), got:\n{q}"
        );
    }

    #[test]
    fn excludes_active_counters_and_placeholders() {
        let q = ai_query();
        // Distributable keys present.
        for key in ["cursor_acceptance", "cc_tool_acceptance", "ai_loc_share2"] {
            assert!(
                q.contains(&format!("'{key}'")),
                "missing distributable key {key}"
            );
        }
        // Member-scale flags + NULL placeholders must NOT be unpivoted.
        for key in [
            "active_ai_members",
            "cursor_active",
            "cc_active",
            "codex_active",
            "chatgpt",
            "claude_web",
        ] {
            assert!(
                !q.contains(&format!("('{key}'")),
                "non-distributable key {key} must be excluded from the ARRAY JOIN"
            );
        }
    }
}
