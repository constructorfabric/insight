//! Un-stub ChatGPT Team (Codex + Chat) in Team / IC Bullet AI `query_ref`s
//! (INSIGHT-459).
//!
//! Pairs with ingestion migration
//! `20260609000000_ai-chatgpt-team-gold.sql`, which adds Branch 4
//! (`tool = 'codex'`) and Branch 5 (`tool = 'chatgpt'`) to
//! `insight.ai_bullet_rows`, emitting:
//!   codex_active, codex_lines, codex_sessions, chatgpt_active, chatgpt.
//!
//! Before this migration the wide-aggregate hardcoded
//! `codex_active_v` / `chatgpt_v` / `claude_web_v` as `CAST(NULL …)`
//! (ComingSoon), so the keys rendered as ComingSoon on the FE.
//!
//! Changes to each `query_ref` (Team + IC):
//!   1. `codex_active_v` ← real `countIf(metric_key='codex_active')` marker
//!      (was hardcoded NULL).
//!   2. `chatgpt_v` ← real `sumIf(metric_value, metric_key='chatgpt')`
//!      (was hardcoded NULL). ChatGPT chat interactions (messages).
//!   3. New `codex_lines_v` / `codex_sessions_v` (`sumIf`, like cc_lines /
//!      cc_sessions) and `chatgpt_active_v` (`countIf` marker, like
//!      codex_active).
//!   4. `chatgpt_active` added to ACTIVE_LIST so its outer aggregation is
//!      `sum(per-person marker)` = count of active persons (DAU), matching
//!      codex_active / cc_active.
//!   5. `claude_web_v` stays hardcoded NULL — Claude web is not collected.
//!
//! Backend-emitted `metric_key`s: 19 → 22.
//! FE: `cyber-insight-front` threshold-config gains entries for the new keys.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const TEAM_BULLET_AI_ID: &str = "00000000000000000001000000000006";
const IC_BULLET_AI_ID: &str = "00000000000000000001000000000013";

/// Active-marker `metric_key`s — outer uses `sum(v_period)` (count of active
/// persons). Adds `chatgpt_active` to the m20260601 list.
const ACTIVE_LIST: &str =
    "'active_ai_members', 'cursor_active', 'cc_active', 'codex_active', 'chatgpt_active'";

/// Inner wide-aggregate: one row per `person_id`, every FE-visible
/// `metric_key` in its own column. codex_active / chatgpt now read real
/// values; codex_lines / codex_sessions / chatgpt_active are new.
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
         if(countIf(metric_key = 'codex_active') > 0, toFloat64(1), CAST(NULL AS Nullable(Float64))) AS codex_active_v, \
         sumIf(metric_value, metric_key = 'chatgpt') AS chatgpt_v, \
         CAST(NULL AS Nullable(Float64)) AS claude_web_v, \
         sumIf(metric_value, metric_key = 'codex_lines') AS codex_lines_v, \
         sumIf(metric_value, metric_key = 'codex_sessions') AS codex_sessions_v, \
         if(countIf(metric_key = 'chatgpt_active') > 0, toFloat64(1), CAST(NULL AS Nullable(Float64))) AS chatgpt_active_v \
     FROM insight.ai_bullet_rows \
     GROUP BY person_id"
}

/// `ARRAY JOIN` unpivot: wide columns → long rows per person.
/// 19 keys from m20260601 + 3 new ChatGPT Team keys = 22 total.
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
    /// `20260609000000_ai-chatgpt-team-gold.sql` first (which removes the
    /// codex_*/chatgpt_* keys from the view), then restore the previous
    /// `query_ref` from `m20260601_000001` manually.
    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom(
            "m20260609_000001_ai_chatgpt_team_metrics is irreversible: \
             roll back the paired CH migration \
             20260609000000_ai-chatgpt-team-gold.sql first."
                .to_string(),
        ))
    }
}
