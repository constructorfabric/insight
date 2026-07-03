//! Honest-null patch for `TEAM_BULLET_AI` (UUID …06) and `IC_BULLET_AI`
//! (UUID …13).
//!
//! The squashed `m20260422_000001_seed_metrics` seeds inline the bullet-rows
//! aggregation logic instead of reading from the pre-aggregated `insight.*_stats`
//! views. Those inline queries hardcode `toFloat64(0)` for median/min and
//! `toFloat64(count())` for max in the active-family metrics
//! (`active_ai_members`, `cursor_active`, `cc_active`, `codex_active`). When
//! the upstream `ai_bullet_rows` view emits NULL for unsourced metrics
//! (Claude Code / Codex / `ChatGPT` / Claude.ai — see CH migration
//! `20260423120000_bullet-views-honest-nulls.sql`), those hardcoded fallbacks
//! still return `median=0, min=0, max=teamsize`, producing a "0 out of N"
//! bullet instead of the `ComingSoon` placeholder the FE renders when
//! `range_min`/`range_max` are NULL.
//!
//! This migration wraps the hardcodes in `if(count(v_period) = 0, NULL, …)`
//! so the synthetic distribution collapses to NULL when the metric has no
//! non-NULL values.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const TEAM_BULLET_AI_ID: &str = "00000000000000000001000000000006";
const IC_BULLET_AI_ID: &str = "00000000000000000001000000000013";

const TEAM_BULLET_AI_QUERY: &str = "SELECT p.metric_key AS metric_key, \
multiIf(p.metric_key IN ('active_ai_members', 'cursor_active', 'cc_active', 'codex_active'), sum(p.v_period), avg(p.v_period)) AS value, \
any(c.company_median) AS median, \
any(c.company_min) AS range_min, \
any(c.company_max) AS range_max \
FROM (\
    SELECT metric_key, person_id, any(org_unit_id) AS org_unit_id, \
    multiIf(\
        metric_key IN ('chatgpt', 'cc_lines', 'cc_sessions', 'cursor_agents', 'cursor_lines', 'claude_web', 'cursor_completions', 'team_ai_loc'), sum(metric_value), \
        metric_key IN ('active_ai_members', 'cursor_active', 'cc_active', 'codex_active'), max(metric_value), \
        avg(metric_value)) AS v_period \
    FROM insight.ai_bullet_rows GROUP BY metric_key, person_id\
) p \
LEFT JOIN (\
    SELECT metric_key, \
    multiIf(metric_key IN ('active_ai_members', 'cursor_active', 'cc_active', 'codex_active'), \
            if(count(v_period) = 0, CAST(NULL AS Nullable(Float64)), toFloat64(0)), \
            quantileExact(0.5)(v_period)) AS company_median, \
    multiIf(metric_key IN ('active_ai_members', 'cursor_active', 'cc_active', 'codex_active'), \
            if(count(v_period) = 0, CAST(NULL AS Nullable(Float64)), toFloat64(0)), \
            min(v_period)) AS company_min, \
    multiIf(metric_key IN ('active_ai_members', 'cursor_active', 'cc_active', 'codex_active'), \
            if(count(v_period) = 0, CAST(NULL AS Nullable(Float64)), toFloat64(count())), \
            max(v_period)) AS company_max \
    FROM (\
        SELECT metric_key, person_id, \
        multiIf(\
            metric_key IN ('chatgpt', 'cc_lines', 'cc_sessions', 'cursor_agents', 'cursor_lines', 'claude_web', 'cursor_completions', 'team_ai_loc'), sum(metric_value), \
            metric_key IN ('active_ai_members', 'cursor_active', 'cc_active', 'codex_active'), max(metric_value), \
            avg(metric_value)) AS v_period \
        FROM insight.ai_bullet_rows GROUP BY metric_key, person_id\
    ) inner_c \
    GROUP BY metric_key\
) c ON c.metric_key = p.metric_key \
GROUP BY p.metric_key";

const IC_BULLET_AI_QUERY: &str = "SELECT p.metric_key AS metric_key, \
multiIf(p.metric_key IN ('active_ai_members', 'cursor_active', 'cc_active', 'codex_active'), sum(p.v_period), avg(p.v_period)) AS value, \
any(c.team_median) AS median, \
any(c.team_min) AS range_min, \
any(c.team_max) AS range_max \
FROM (\
    SELECT metric_key, person_id, any(org_unit_id) AS org_unit_id, \
    multiIf(\
        metric_key IN ('chatgpt', 'cc_lines', 'cc_sessions', 'cursor_agents', 'cursor_lines', 'claude_web', 'cursor_completions', 'team_ai_loc'), sum(metric_value), \
        metric_key IN ('active_ai_members', 'cursor_active', 'cc_active', 'codex_active'), max(metric_value), \
        avg(metric_value)) AS v_period \
    FROM insight.ai_bullet_rows GROUP BY metric_key, person_id\
) p \
LEFT JOIN (\
    SELECT metric_key, org_unit_id, \
    multiIf(metric_key IN ('active_ai_members', 'cursor_active', 'cc_active', 'codex_active'), \
            if(count(v_period) = 0, CAST(NULL AS Nullable(Float64)), toFloat64(0)), \
            quantileExact(0.5)(v_period)) AS team_median, \
    multiIf(metric_key IN ('active_ai_members', 'cursor_active', 'cc_active', 'codex_active'), \
            if(count(v_period) = 0, CAST(NULL AS Nullable(Float64)), toFloat64(0)), \
            min(v_period)) AS team_min, \
    multiIf(metric_key IN ('active_ai_members', 'cursor_active', 'cc_active', 'codex_active'), \
            if(count(v_period) = 0, CAST(NULL AS Nullable(Float64)), toFloat64(count())), \
            max(v_period)) AS team_max \
    FROM (\
        SELECT metric_key, person_id, any(org_unit_id) AS org_unit_id, \
        multiIf(\
            metric_key IN ('chatgpt', 'cc_lines', 'cc_sessions', 'cursor_agents', 'cursor_lines', 'claude_web', 'cursor_completions', 'team_ai_loc'), sum(metric_value), \
            metric_key IN ('active_ai_members', 'cursor_active', 'cc_active', 'codex_active'), max(metric_value), \
            avg(metric_value)) AS v_period \
        FROM insight.ai_bullet_rows GROUP BY metric_key, person_id\
    ) inner_c \
    GROUP BY metric_key, org_unit_id\
) c ON c.metric_key = p.metric_key AND c.org_unit_id = p.org_unit_id \
GROUP BY p.metric_key";

// Snapshotted from m20260422_000001_seed_metrics so `down()` can restore the
// exact pre-patch text on rollback. If you edit the upstream seed, refresh
// these mirrors too.
const TEAM_BULLET_AI_QUERY_OLD: &str = "SELECT p.metric_key AS metric_key, multiIf(p.metric_key IN ('active_ai_members', 'cursor_active', 'cc_active', 'codex_active'), sum(p.v_period), avg(p.v_period)) AS value, any(c.company_median) AS median, any(c.company_min) AS range_min, any(c.company_max) AS range_max FROM (SELECT metric_key, person_id, any(org_unit_id) AS org_unit_id, multiIf(metric_key IN ('chatgpt', 'cc_lines', 'cc_sessions', 'cursor_agents', 'cursor_lines', 'claude_web', 'cursor_completions', 'team_ai_loc'), sum(metric_value), metric_key IN ('active_ai_members', 'cursor_active', 'cc_active', 'codex_active'), max(metric_value), avg(metric_value)) AS v_period FROM insight.ai_bullet_rows GROUP BY metric_key, person_id) p LEFT JOIN (SELECT metric_key, multiIf(metric_key IN ('active_ai_members', 'cursor_active', 'cc_active', 'codex_active'), toFloat64(0), quantileExact(0.5)(v_period)) AS company_median, multiIf(metric_key IN ('active_ai_members', 'cursor_active', 'cc_active', 'codex_active'), toFloat64(0), min(v_period)) AS company_min, multiIf(metric_key IN ('active_ai_members', 'cursor_active', 'cc_active', 'codex_active'), toFloat64(count()), max(v_period)) AS company_max FROM (SELECT metric_key, person_id, multiIf(metric_key IN ('chatgpt', 'cc_lines', 'cc_sessions', 'cursor_agents', 'cursor_lines', 'claude_web', 'cursor_completions', 'team_ai_loc'), sum(metric_value), metric_key IN ('active_ai_members', 'cursor_active', 'cc_active', 'codex_active'), max(metric_value), avg(metric_value)) AS v_period FROM insight.ai_bullet_rows GROUP BY metric_key, person_id) inner_c GROUP BY metric_key) c ON c.metric_key = p.metric_key GROUP BY p.metric_key";

const IC_BULLET_AI_QUERY_OLD: &str = "SELECT p.metric_key AS metric_key, multiIf(p.metric_key IN ('active_ai_members', 'cursor_active', 'cc_active', 'codex_active'), sum(p.v_period), avg(p.v_period)) AS value, any(c.team_median) AS median, any(c.team_min) AS range_min, any(c.team_max) AS range_max FROM (SELECT metric_key, person_id, any(org_unit_id) AS org_unit_id, multiIf(metric_key IN ('chatgpt', 'cc_lines', 'cc_sessions', 'cursor_agents', 'cursor_lines', 'claude_web', 'cursor_completions', 'team_ai_loc'), sum(metric_value), metric_key IN ('active_ai_members', 'cursor_active', 'cc_active', 'codex_active'), max(metric_value), avg(metric_value)) AS v_period FROM insight.ai_bullet_rows GROUP BY metric_key, person_id) p LEFT JOIN (SELECT metric_key, org_unit_id, multiIf(metric_key IN ('active_ai_members', 'cursor_active', 'cc_active', 'codex_active'), toFloat64(0), quantileExact(0.5)(v_period)) AS team_median, multiIf(metric_key IN ('active_ai_members', 'cursor_active', 'cc_active', 'codex_active'), toFloat64(0), min(v_period)) AS team_min, multiIf(metric_key IN ('active_ai_members', 'cursor_active', 'cc_active', 'codex_active'), toFloat64(count()), max(v_period)) AS team_max FROM (SELECT metric_key, person_id, any(org_unit_id) AS org_unit_id, multiIf(metric_key IN ('chatgpt', 'cc_lines', 'cc_sessions', 'cursor_agents', 'cursor_lines', 'claude_web', 'cursor_completions', 'team_ai_loc'), sum(metric_value), metric_key IN ('active_ai_members', 'cursor_active', 'cc_active', 'codex_active'), max(metric_value), avg(metric_value)) AS v_period FROM insight.ai_bullet_rows GROUP BY metric_key, person_id) inner_c GROUP BY metric_key, org_unit_id) c ON c.metric_key = p.metric_key AND c.org_unit_id = p.org_unit_id GROUP BY p.metric_key";

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();

        for (hex_id, query) in [
            (TEAM_BULLET_AI_ID, TEAM_BULLET_AI_QUERY),
            (IC_BULLET_AI_ID, IC_BULLET_AI_QUERY),
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
            (TEAM_BULLET_AI_ID, TEAM_BULLET_AI_QUERY_OLD),
            (IC_BULLET_AI_ID, IC_BULLET_AI_QUERY_OLD),
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
