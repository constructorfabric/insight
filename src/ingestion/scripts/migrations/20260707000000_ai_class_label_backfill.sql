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
-- Historical STAGING rows keep '' labels (staging tables may not exist when
-- this runs, and the incremental class models never re-read old staging
-- rows). A manual class rebuild must therefore full-refresh the staging
-- models together with the class tables so labels re-derive from the
-- staging model literals.

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
