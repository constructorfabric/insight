CREATE DATABASE IF NOT EXISTS `insight`;

CREATE TABLE IF NOT EXISTS insight.collab_metric_observations
(
    `tenant_id` String,
    `source_key` String,
    `entity_type` String,
    `entity_id` String,
    `metric_date` Date,
    `observed_at` Nullable(DateTime64(3)),
    `measure_key` String,
    `value` Nullable(Float64),
    `subject_key` Nullable(String),
    `dimensions` Array(Tuple(
        key String,
        value String,
        label Nullable(String)))
)
ENGINE = MergeTree
ORDER BY (source_key, measure_key, entity_id, metric_date)
SETTINGS replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS insight.task_issue_state
(
    `tenant_id` Nullable(String),
    `entity_id` Nullable(String),
    `insight_source_id` String,
    `issue_id` String,
    `status_category` String,
    `issue_type` String,
    `due_date` Nullable(Date),
    `time_estimate_seconds` Nullable(Float64),
    `time_spent_seconds` Nullable(Float64),
    `created_at` DateTime64(3),
    `final_close_at` Nullable(DateTime64(3)),
    `last_status_event_at` DateTime64(3)
)
ENGINE = MergeTree
ORDER BY (insight_source_id, issue_id)
SETTINGS replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS insight.task_metric_observations
(
    `tenant_id` String,
    `source_key` String,
    `entity_type` String,
    `entity_id` String,
    `metric_date` Date,
    `observed_at` Nullable(DateTime64(3)),
    `measure_key` String,
    `value` Nullable(Float64),
    `subject_key` Nullable(String),
    `dimensions` Array(Tuple(
        key String,
        value String,
        label Nullable(String)))
)
ENGINE = MergeTree
ORDER BY (source_key, measure_key, entity_id, metric_date)
SETTINGS replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS insight.task_status_spans
(
    `insight_source_id` String,
    `issue_id` String,
    `interval_start` DateTime64(3),
    `interval_end` DateTime64(3),
    `status_category` String,
    `duration_seconds` Float64
)
ENGINE = MergeTree
ORDER BY (insight_source_id, issue_id, interval_start)
SETTINGS replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS insight.wiki_metric_observations
(
    `tenant_id` String,
    `source_key` String,
    `entity_type` String,
    `entity_id` String,
    `metric_date` Date,
    `observed_at` Nullable(DateTime64(3)),
    `measure_key` String,
    `value` Nullable(Float64),
    `subject_key` Nullable(String),
    `dimensions` Array(Tuple(
        key String,
        value String,
        label Nullable(String)))
)
ENGINE = MergeTree
ORDER BY (source_key, measure_key, entity_id, metric_date)
SETTINGS replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE OR REPLACE VIEW insight.ai_metric_observations
(
    `tenant_id` String,
    `source_key` String,
    `entity_type` String,
    `entity_id` String,
    `metric_date` Date,
    `observed_at` Nullable(DateTime64(3)),
    `measure_key` String,
    `value` Nullable(Float64),
    `subject_key` Nullable(String),
    `dimensions` Array(Tuple(
        key String,
        value String,
        label Nullable(String)))
)
AS WITH
    ai_dev_usage_source AS
    (
        SELECT
            insight_tenant_id AS tenant_id,
            lower(email) AS entity_id,
            day AS metric_date,
            CAST([('tool', tool, multiIf(tool = 'cursor', 'Cursor', tool = 'claude_code', 'Claude Code', tool = 'copilot', 'GitHub Copilot', tool = 'codex', 'Codex', tool = 'claude', 'Claude', tool = 'chatgpt', 'ChatGPT', tool))], 'Array(Tuple(key String, value String, label Nullable(String)))') AS tool_dimensions,
            conversation_count,
            lines_added,
            lines_removed,
            tool_use_offered,
            tool_use_accepted,
            cost_cents
        FROM silver.class_ai_dev_usage
        WHERE (email IS NOT NULL) AND (email != '')
    ),
    ai_assistant_usage_source AS
    (
        SELECT
            insight_tenant_id AS tenant_id,
            lower(email) AS entity_id,
            day AS metric_date,
            surface,
            CAST([('tool', tool, multiIf(tool = 'cursor', 'Cursor', tool = 'claude_code', 'Claude Code', tool = 'copilot', 'GitHub Copilot', tool = 'codex', 'Codex', tool = 'claude', 'Claude', tool = 'chatgpt', 'ChatGPT', tool))], 'Array(Tuple(key String, value String, label Nullable(String)))') AS tool_dimensions,
            CAST([('tool', tool, multiIf(tool = 'cursor', 'Cursor', tool = 'claude_code', 'Claude Code', tool = 'copilot', 'GitHub Copilot', tool = 'codex', 'Codex', tool = 'claude', 'Claude', tool = 'chatgpt', 'ChatGPT', tool)), ('surface', surface, multiIf(surface = 'chat', 'Chat', surface = 'excel', 'Excel', surface = 'powerpoint', 'PowerPoint', surface = 'cowork', 'Cowork', surface = 'cross', 'Cross', surface))], 'Array(Tuple(key String, value String, label Nullable(String)))') AS tool_surface_dimensions,
            conversation_count,
            message_count,
            action_count,
            cost_cents
        FROM silver.class_ai_assistant_usage
        WHERE (email IS NOT NULL) AND (email != '')
    ),
    measure_observations AS
    (
        SELECT
            tenant_id,
            entity_id,
            metric_date,
            'accepted_lines' AS measure_key,
            toNullable(sumIf(toFloat64(lines_added), lines_added IS NOT NULL)) AS value,
            tool_dimensions AS dimensions
        FROM ai_dev_usage_source
        GROUP BY
            tenant_id,
            entity_id,
            metric_date,
            tool_dimensions
        HAVING countIf(lines_added IS NOT NULL) > 0
        UNION ALL
        SELECT
            tenant_id,
            entity_id,
            metric_date,
            'removed_lines' AS measure_key,
            toNullable(sumIf(toFloat64(lines_removed), lines_removed IS NOT NULL)) AS value,
            tool_dimensions AS dimensions
        FROM ai_dev_usage_source
        GROUP BY
            tenant_id,
            entity_id,
            metric_date,
            tool_dimensions
        HAVING countIf(lines_removed IS NOT NULL) > 0
        UNION ALL
        SELECT
            tenant_id,
            entity_id,
            metric_date,
            'active_day' AS measure_key,
            toNullable(toFloat64(1)) AS value,
            CAST([], 'Array(Tuple(key String, value String, label Nullable(String)))') AS dimensions
        FROM
        (
            SELECT DISTINCT
                tenant_id,
                entity_id,
                metric_date
            FROM ai_dev_usage_source
            UNION DISTINCT
            SELECT DISTINCT
                tenant_id,
                entity_id,
                metric_date
            FROM ai_assistant_usage_source
        )
        UNION ALL
        SELECT
            tenant_id,
            entity_id,
            metric_date,
            'cost_usd' AS measure_key,
            toNullable(sumIf(toFloat64(cost_cents / 100), (cost_cents / 100) IS NOT NULL)) AS value,
            tool_dimensions AS dimensions
        FROM ai_dev_usage_source
        GROUP BY
            tenant_id,
            entity_id,
            metric_date,
            tool_dimensions
        HAVING countIf((cost_cents / 100) IS NOT NULL) > 0
        UNION ALL
        SELECT
            tenant_id,
            entity_id,
            metric_date,
            'cost_usd' AS measure_key,
            toNullable(sumIf(toFloat64(cost_cents / 100), (cost_cents / 100) IS NOT NULL)) AS value,
            tool_dimensions AS dimensions
        FROM ai_assistant_usage_source
        GROUP BY
            tenant_id,
            entity_id,
            metric_date,
            tool_dimensions
        HAVING countIf((cost_cents / 100) IS NOT NULL) > 0
        UNION ALL
        SELECT
            tenant_id,
            entity_id,
            metric_date,
            'accepted_edit_actions' AS measure_key,
            toNullable(sumIf(toFloat64(tool_use_accepted), tool_use_accepted IS NOT NULL)) AS value,
            tool_dimensions AS dimensions
        FROM ai_dev_usage_source
        GROUP BY
            tenant_id,
            entity_id,
            metric_date,
            tool_dimensions
        HAVING countIf(tool_use_accepted IS NOT NULL) > 0
        UNION ALL
        SELECT
            tenant_id,
            entity_id,
            metric_date,
            'tool_use_offered' AS measure_key,
            toNullable(sumIf(toFloat64(tool_use_offered), tool_use_offered IS NOT NULL)) AS value,
            tool_dimensions AS dimensions
        FROM ai_dev_usage_source
        GROUP BY
            tenant_id,
            entity_id,
            metric_date,
            tool_dimensions
        HAVING countIf(tool_use_offered IS NOT NULL) > 0
        UNION ALL
        SELECT
            tenant_id,
            entity_id,
            metric_date,
            'dev_conversations' AS measure_key,
            toNullable(sumIf(toFloat64(conversation_count), conversation_count IS NOT NULL)) AS value,
            tool_dimensions AS dimensions
        FROM ai_dev_usage_source
        GROUP BY
            tenant_id,
            entity_id,
            metric_date,
            tool_dimensions
        HAVING countIf(conversation_count IS NOT NULL) > 0
        UNION ALL
        SELECT
            tenant_id,
            entity_id,
            metric_date,
            'assistant_messages' AS measure_key,
            toNullable(sumIf(toFloat64(message_count), message_count IS NOT NULL)) AS value,
            tool_surface_dimensions AS dimensions
        FROM ai_assistant_usage_source
        GROUP BY
            tenant_id,
            entity_id,
            metric_date,
            tool_surface_dimensions
        HAVING countIf(message_count IS NOT NULL) > 0
        UNION ALL
        SELECT
            tenant_id,
            entity_id,
            metric_date,
            'assistant_actions' AS measure_key,
            toNullable(sumIf(toFloat64(action_count), action_count IS NOT NULL)) AS value,
            tool_surface_dimensions AS dimensions
        FROM ai_assistant_usage_source
        GROUP BY
            tenant_id,
            entity_id,
            metric_date,
            tool_surface_dimensions
        HAVING countIf(action_count IS NOT NULL) > 0
        UNION ALL
        SELECT
            tenant_id,
            entity_id,
            metric_date,
            'chat_assistant_conversations' AS measure_key,
            toNullable(sumIf(toFloat64(conversation_count), conversation_count IS NOT NULL)) AS value,
            tool_surface_dimensions AS dimensions
        FROM ai_assistant_usage_source
        WHERE surface = 'chat'
        GROUP BY
            tenant_id,
            entity_id,
            metric_date,
            tool_surface_dimensions
        HAVING countIf(conversation_count IS NOT NULL) > 0
    )
SELECT
    assumeNotNull(tenant_id) AS tenant_id,
    'ai_usage' AS source_key,
    'person' AS entity_type,
    assumeNotNull(entity_id) AS entity_id,
    assumeNotNull(metric_date) AS metric_date,
    CAST(NULL, 'Nullable(DateTime64(3))') AS observed_at,
    measure_key,
    value,
    CAST(NULL, 'Nullable(String)') AS subject_key,
    dimensions
FROM measure_observations
WHERE (tenant_id IS NOT NULL) AND (entity_id IS NOT NULL) AND (metric_date IS NOT NULL)
;

CREATE OR REPLACE VIEW insight.metric_entity_cohorts_current
(
    `tenant_id` String,
    `entity_type` String,
    `entity_id` String,
    `cohort_key` String,
    `cohort_id` Nullable(String)
)
AS SELECT
    assumeNotNull(tenant_id) AS tenant_id,
    'person' AS entity_type,
    assumeNotNull(entity_id) AS entity_id,
    'org_unit' AS cohort_key,
    cohort_id
FROM
(
    SELECT
        workspace_id AS tenant_id,
        lower(assumeNotNull(email)) AS entity_id,
        coalesce(nullIf(toString(org_unit_id), ''), nullIf(department_name, '')) AS cohort_id
    FROM silver.class_people
    WHERE (email IS NOT NULL) AND (email != '') AND (workspace_id IS NOT NULL) AND (workspace_id != '')
    ORDER BY
        tenant_id ASC,
        entity_id ASC,
        coalesce(parseDateTimeBestEffortOrNull(toString(valid_from)), toDateTime('1970-01-01')) DESC,
        unique_key DESC
    LIMIT 1 BY
        tenant_id,
        entity_id
)
WHERE (tenant_id IS NOT NULL) AND (tenant_id != '') AND (entity_id IS NOT NULL) AND (entity_id != '')
;

