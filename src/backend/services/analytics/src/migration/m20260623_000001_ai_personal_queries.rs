use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const ZERO_TENANT: &str = "00000000000000000000000000000000";
const AI_PERSONAL_HEX: &str = "00000000000000000001000000000050";
const AI_PERSONAL_TREND_HEX: &str = "00000000000000000001000000000051";
const AI_PERSONAL_COUNTERS_HEX: &str = "00000000000000000001000000000052";

const AI_TOOL_NAME_SQL: &str = "multiIf(tool = 'claude_code', 'Claude Code', tool = 'cursor', 'Cursor', tool = 'codex', 'Codex', tool = 'copilot', 'GitHub Copilot', tool = 'windsurf', 'Windsurf', tool)";
const AI_PERSON_COUNTER_VALUES_QR: &str = r"SELECT metric_values.person_id AS person_id, p.org_unit_id AS org_unit_id, metric_values.metric_key AS metric_key, metric_values.value AS value FROM (SELECT person_id, kv.1 AS metric_key, kv.2 AS value FROM (SELECT person_id, if(countIf(ai_accepted_lines IS NOT NULL) > 0, sumIf(ai_accepted_lines, ai_accepted_lines IS NOT NULL), CAST(NULL AS Nullable(Float64))) AS ai_accepted_lines, if(countIf(ai_removed_lines IS NOT NULL) > 0, sumIf(ai_removed_lines, ai_removed_lines IS NOT NULL), CAST(NULL AS Nullable(Float64))) AS ai_removed_lines, if(countIf(ai_active_days IS NOT NULL) > 0, sumIf(ai_active_days, ai_active_days IS NOT NULL), CAST(NULL AS Nullable(Float64))) AS ai_active_days, if(countIf(ai_cost_cents IS NOT NULL) > 0, sumIf(ai_cost_cents, ai_cost_cents IS NOT NULL) / 100, CAST(NULL AS Nullable(Float64))) AS ai_cost, if(countIf(ai_accepted_edit_actions IS NOT NULL) > 0, sumIf(ai_accepted_edit_actions, ai_accepted_edit_actions IS NOT NULL), CAST(NULL AS Nullable(Float64))) AS ai_accepted_edit_actions, if(sumIf(ai_tool_acceptance_offered, ai_tool_acceptance_offered IS NOT NULL) > 0, 100 * sumIf(ai_tool_acceptance_accepted, ai_tool_acceptance_accepted IS NOT NULL) / sumIf(ai_tool_acceptance_offered, ai_tool_acceptance_offered IS NOT NULL), CAST(NULL AS Nullable(Float64))) AS ai_tool_acceptance_rate, if(countIf(ai_assistant_messages IS NOT NULL) > 0, sumIf(ai_assistant_messages, ai_assistant_messages IS NOT NULL), CAST(NULL AS Nullable(Float64))) AS ai_assistant_messages, if(countIf(ai_assistant_actions IS NOT NULL) > 0, sumIf(ai_assistant_actions, ai_assistant_actions IS NOT NULL), CAST(NULL AS Nullable(Float64))) AS ai_assistant_actions, if(countIf(ai_dev_agent_conversations IS NOT NULL) > 0, sumIf(ai_dev_agent_conversations, ai_dev_agent_conversations IS NOT NULL), CAST(NULL AS Nullable(Float64))) AS ai_dev_agent_conversations, if(countIf(ai_chat_assistant_conversations IS NOT NULL) > 0, sumIf(ai_chat_assistant_conversations, ai_chat_assistant_conversations IS NOT NULL), CAST(NULL AS Nullable(Float64))) AS ai_chat_assistant_conversations FROM insight.ai_person_counter_daily GROUP BY person_id) d ARRAY JOIN [('ai_person_counter_daily.ai_accepted_lines', ai_accepted_lines), ('ai_person_counter_daily.ai_removed_lines', ai_removed_lines), ('ai_person_counter_daily.ai_active_days', ai_active_days), ('ai_person_counter_daily.ai_cost', ai_cost), ('ai_person_counter_daily.ai_accepted_edit_actions', ai_accepted_edit_actions), ('ai_person_counter_daily.ai_tool_acceptance_rate', ai_tool_acceptance_rate), ('ai_person_counter_daily.ai_assistant_messages', ai_assistant_messages), ('ai_person_counter_daily.ai_assistant_actions', ai_assistant_actions), ('ai_person_counter_daily.ai_dev_agent_conversations', ai_dev_agent_conversations), ('ai_person_counter_daily.ai_chat_assistant_conversations', ai_chat_assistant_conversations)] AS kv WHERE kv.2 IS NOT NULL) metric_values LEFT JOIN insight.people AS p ON metric_values.person_id = p.person_id";

fn ai_personal_qr() -> String {
    format!(
        "SELECT person_id, tool, {AI_TOOL_NAME_SQL} AS tool_name, sum(accepted_lines_added) AS accepted_lines_added, sum(accepted_lines_removed) AS accepted_lines_removed, if(countIf(cost_cents IS NOT NULL) > 0, sumIf(cost_cents, cost_cents IS NOT NULL), CAST(NULL AS Nullable(Float64))) AS cost_cents, uniqExact(metric_date) AS active_days FROM insight.ai_dev_tool_daily GROUP BY person_id, tool"
    )
}

fn ai_personal_trend_qr() -> String {
    format!(
        "SELECT person_id, metric_date, tool, {AI_TOOL_NAME_SQL} AS tool_name, sum(accepted_lines_added) AS accepted_lines_added FROM insight.ai_dev_tool_trend GROUP BY person_id, metric_date, tool"
    )
}

fn ai_personal_counters_qr() -> String {
    format!(
        "SELECT p.person_id AS person_id, p.org_unit_id AS org_unit_id, p.metric_key AS metric_key, p.value AS value, c.team_median AS median, c.team_p25 AS p25, c.team_p75 AS p75, c.team_n AS n, c.team_min AS range_min, c.team_max AS range_max FROM ({AI_PERSON_COUNTER_VALUES_QR}) p LEFT JOIN (SELECT metric_key, org_unit_id, quantileExact(0.5)(value) AS team_median, quantileExact(0.25)(value) AS team_p25, quantileExact(0.75)(value) AS team_p75, toFloat64(count()) AS team_n, min(value) AS team_min, max(value) AS team_max FROM ({AI_PERSON_COUNTER_VALUES_QR}) person_values GROUP BY metric_key, org_unit_id) c ON c.metric_key = p.metric_key AND c.org_unit_id = p.org_unit_id"
    )
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        let ai_personal_counters_qr = ai_personal_counters_qr();
        let ai_personal_qr = ai_personal_qr();
        let ai_personal_trend_qr = ai_personal_trend_qr();
        for (hex, name, description, query) in [
            (
                AI_PERSONAL_HEX,
                "AI Tool Summary",
                "Per-person AI tool summary rows.",
                ai_personal_qr,
            ),
            (
                AI_PERSONAL_TREND_HEX,
                "AI Tool Trend",
                "Per-person daily AI accepted-lines rows by tool.",
                ai_personal_trend_qr,
            ),
            (
                AI_PERSONAL_COUNTERS_HEX,
                "AI Personal Peer Counters",
                "Per-person AI peer counter rows.",
                ai_personal_counters_qr,
            ),
        ] {
            db.execute_unprepared(&format!(
                "INSERT INTO metrics (id, insight_tenant_id, name, description, query_ref, is_enabled) \
                 VALUES (UNHEX('{hex}'), UNHEX('{ZERO_TENANT}'), '{name}', '{description}', '{qr}', 1) \
                 ON DUPLICATE KEY UPDATE name=VALUES(name), description=VALUES(description), query_ref=VALUES(query_ref), is_enabled=1",
                qr = query.replace('\'', "''"),
            ))
            .await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(&format!(
                "DELETE FROM metrics WHERE id IN (UNHEX('{AI_PERSONAL_HEX}'), UNHEX('{AI_PERSONAL_TREND_HEX}'), UNHEX('{AI_PERSONAL_COUNTERS_HEX}'))"
            ))
            .await?;
        Ok(())
    }
}
