-- Ensure the AI class tables carry the gold-contract columns the unified
-- metrics gold model reads, and backfill connector-declared labels.
--
-- The gold model (insight.ai_metric_observations) is built at deploy time,
-- before any connector re-syncs. It reads class-contract columns that the
-- rework introduced. On an existing install the class tables are real
-- (not placeholders), so create-bronze-placeholders.sh — which reconciles
-- these columns only on placeholder-marked tables — never touches them, and
-- the full-refresh that a connector major-bump dispatches has not run yet.
-- The gold `dbt run` therefore fails on a missing column unless the column
-- exists at deploy time. This migration adds the columns unconditionally so
-- the schema is present immediately; their VALUES arrive later:
--   * labels are declared constants — backfilled in place below;
--   * conversation_count is source data — stays NULL until the connector
--     full-refresh re-materializes the class table from Bronze.
--
-- Idempotent: ADD COLUMN IF NOT EXISTS; UPDATE re-runs match zero rows.
--
-- Historical STAGING rows get the label repair from the guarded step in
-- apply-ch-migrations.sh (staging tables may not exist on fresh installs,
-- so their repair cannot live in this unconditional channel). Class rows
-- are repaired here directly so instances whose connectors never trigger a
-- full re-materialization still converge.

ALTER TABLE silver.class_ai_dev_usage ADD COLUMN IF NOT EXISTS conversation_count Nullable(Float64);
ALTER TABLE silver.class_ai_dev_usage ADD COLUMN IF NOT EXISTS tool_label String DEFAULT '';

ALTER TABLE silver.class_ai_dev_usage
    UPDATE tool_label = multiIf(
        tool = 'cursor', 'Cursor',
        tool = 'claude_code', 'Claude Code',
        tool = 'copilot', 'GitHub Copilot',
        tool = 'codex', 'Codex',
        tool
    )
    WHERE tool_label = ''
    SETTINGS mutations_sync = 2;

ALTER TABLE silver.class_ai_assistant_usage ADD COLUMN IF NOT EXISTS tool_label String DEFAULT '';
ALTER TABLE silver.class_ai_assistant_usage ADD COLUMN IF NOT EXISTS surface_label String DEFAULT '';

ALTER TABLE silver.class_ai_assistant_usage
    UPDATE tool_label = multiIf(
        tool = 'claude', 'Claude',
        tool = 'chatgpt', 'ChatGPT',
        tool
    )
    WHERE tool_label = ''
    SETTINGS mutations_sync = 2;

ALTER TABLE silver.class_ai_assistant_usage
    UPDATE surface_label = multiIf(
        surface = 'chat', 'Chat',
        surface = 'excel', 'Excel',
        surface = 'powerpoint', 'PowerPoint',
        surface = 'cowork', 'Cowork',
        surface = 'cross', 'Cross',
        surface
    )
    WHERE surface_label = ''
    SETTINGS mutations_sync = 2;
