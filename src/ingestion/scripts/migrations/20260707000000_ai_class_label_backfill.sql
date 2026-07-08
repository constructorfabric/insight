-- Backfill connector-declared label columns on the AI class tables.
--
-- Rows ingested before the label columns existed read them as '' (String
-- DEFAULT materialized by on_schema_change='append_new_columns'), while the
-- class contract (silver/ai/schema.yml) requires non-empty labels and Gold
-- consumes them verbatim. The mappings below freeze the labels the staging
-- models declare; new rows are labeled at staging and never match the WHERE.
-- Unknown discriminator values fall back to the value itself so the
-- non-empty contract holds for every row.
--
-- Idempotent: re-runs match zero rows.
--
-- Historical STAGING rows get the same repair from the guarded step in
-- apply-ch-migrations.sh (staging tables may not exist on fresh installs,
-- so their repair cannot live in this unconditional channel). Class rows
-- are repaired here directly so instances whose connectors never trigger a
-- full re-materialization still converge.

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
