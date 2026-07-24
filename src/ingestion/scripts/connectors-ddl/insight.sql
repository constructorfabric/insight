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

CREATE TABLE IF NOT EXISTS insight.git_metric_observations
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

CREATE OR REPLACE VIEW insight.ai_assistant_tool_daily
(
    `person_id` Nullable(String),
    `org_unit_id` Nullable(String),
    `metric_date` Nullable(Date),
    `tool` String,
    `surface` String,
    `source` String,
    `source_id` Nullable(String),
    `assistant_messages` Nullable(Float64),
    `assistant_actions` Nullable(Float64),
    `chat_assistant_conversations` Nullable(Float64)
)
AS SELECT
    lower(a.email) AS person_id,
    p.org_unit_id AS org_unit_id,
    a.day AS metric_date,
    a.tool AS tool,
    a.surface AS surface,
    a.source AS source,
    a.source_id AS source_id,
    if(countIf(a.message_count IS NOT NULL) > 0, sumIf(toFloat64(a.message_count), a.message_count IS NOT NULL), CAST(NULL, 'Nullable(Float64)')) AS assistant_messages,
    if(countIf(a.action_count IS NOT NULL) > 0, sumIf(toFloat64(a.action_count), a.action_count IS NOT NULL), CAST(NULL, 'Nullable(Float64)')) AS assistant_actions,
    if((a.surface = 'chat') AND (countIf(a.conversation_count IS NOT NULL) > 0), sumIf(toFloat64(a.conversation_count), a.conversation_count IS NOT NULL), CAST(NULL, 'Nullable(Float64)')) AS chat_assistant_conversations
FROM silver.class_ai_assistant_usage AS a
LEFT JOIN insight.people AS p ON lower(a.email) = p.person_id
WHERE (a.email IS NOT NULL) AND (a.email != '')
GROUP BY
    person_id,
    org_unit_id,
    metric_date,
    tool,
    surface,
    source,
    source_id
;

CREATE OR REPLACE VIEW insight.ai_bullet_rows
(
    `person_id` Nullable(String),
    `org_unit_id` Nullable(String),
    `metric_date` Nullable(Date),
    `metric_key` String,
    `metric_value` Nullable(Float64)
)
AS SELECT
    lower(c.email) AS person_id,
    p.org_unit_id AS org_unit_id,
    c.day AS metric_date,
    kv.1 AS metric_key,
    kv.2 AS metric_value
FROM silver.class_ai_dev_usage AS c
LEFT JOIN insight.people AS p ON lower(c.email) = p.person_id
ARRAY JOIN [('active_ai_members', toFloat64(1)), ('team_ai_loc', toFloat64(coalesce(c.lines_added, 0)))] AS kv
WHERE (c.email IS NOT NULL) AND (c.email != '')
UNION ALL
SELECT
    lower(c.email) AS person_id,
    p.org_unit_id AS org_unit_id,
    c.day AS metric_date,
    kv.1 AS metric_key,
    kv.2 AS metric_value
FROM silver.class_ai_dev_usage AS c
LEFT JOIN insight.people AS p ON lower(c.email) = p.person_id
ARRAY JOIN [('cursor_active', toFloat64(1)), ('cursor_completions', toFloat64(coalesce(c.tool_use_accepted, 0))), ('cursor_agents', toFloat64(coalesce(c.agent_sessions, 0))), ('cursor_lines', toFloat64(coalesce(c.lines_added, 0))), ('cursor_offered', toFloat64(coalesce(c.tool_use_offered, 0))), ('cursor_total_lines', toFloat64(coalesce(c.total_lines_added, 0)))] AS kv
WHERE (c.tool = 'cursor') AND (c.email IS NOT NULL) AND (c.email != '')
UNION ALL
SELECT
    lower(c.email) AS person_id,
    p.org_unit_id AS org_unit_id,
    c.day AS metric_date,
    kv.1 AS metric_key,
    kv.2 AS metric_value
FROM silver.class_ai_dev_usage AS c
LEFT JOIN insight.people AS p ON lower(c.email) = p.person_id
ARRAY JOIN [('cc_active', toFloat64(1)), ('cc_sessions', toFloat64(coalesce(c.session_count, 0))), ('cc_lines', toFloat64(coalesce(c.lines_added, 0))), ('cc_tool_accept', toFloat64(coalesce(c.tool_use_accepted, 0))), ('cc_offered', toFloat64(coalesce(c.tool_use_offered, 0))), ('cc_cost', toFloat64(coalesce(c.cost_cents, 0)))] AS kv
WHERE (c.tool = 'claude_code') AND (c.email IS NOT NULL) AND (c.email != '')
UNION ALL
SELECT
    lower(c.email) AS person_id,
    p.org_unit_id AS org_unit_id,
    c.day AS metric_date,
    kv.1 AS metric_key,
    kv.2 AS metric_value
FROM silver.class_ai_dev_usage AS c
LEFT JOIN insight.people AS p ON lower(c.email) = p.person_id
ARRAY JOIN [('codex_active', toFloat64(1)), ('codex_lines', toFloat64(coalesce(c.lines_added, 0))), ('codex_sessions', toFloat64(coalesce(c.session_count, 0)))] AS kv
WHERE (c.tool = 'codex') AND (c.email IS NOT NULL) AND (c.email != '')
UNION ALL
SELECT
    lower(a.email) AS person_id,
    p.org_unit_id AS org_unit_id,
    a.day AS metric_date,
    kv.1 AS metric_key,
    kv.2 AS metric_value
FROM silver.class_ai_assistant_usage AS a
LEFT JOIN insight.people AS p ON lower(a.email) = p.person_id
ARRAY JOIN [('chatgpt_active', toFloat64(1)), ('chatgpt', toFloat64(coalesce(a.message_count, 0)))] AS kv
WHERE (a.tool = 'chatgpt') AND (a.email IS NOT NULL) AND (a.email != '')
UNION ALL
SELECT
    lower(o.email) AS person_id,
    p.org_unit_id AS org_unit_id,
    toDate(o.collected_at) AS metric_date,
    'cc_overage' AS metric_key,
    toFloat64(o.overage_cents) AS metric_value
FROM silver.class_ai_overage AS o
LEFT JOIN insight.people AS p ON lower(o.email) = p.person_id
WHERE (o.source = 'claude_team') AND (o.email IS NOT NULL) AND (o.email != '') AND (o.overage_cents IS NOT NULL)
;

CREATE OR REPLACE VIEW insight.ai_company_stats
(
    `metric_key` String,
    `company_value` Nullable(Float64),
    `company_median` Nullable(Float64),
    `company_p5` Nullable(Float64),
    `company_p95` Nullable(Float64)
)
AS SELECT
    metric_key,
    multiIf(metric_key IN ('active_ai_members', 'cursor_active', 'cc_active', 'codex_active'), sum(v), avg(v)) AS company_value,
    multiIf(metric_key IN ('active_ai_members', 'cursor_active', 'cc_active', 'codex_active'), if(count(v) = 0, CAST(NULL, 'Nullable(Float64)'), toFloat64(0)), quantileExact(0.5)(v)) AS company_median,
    multiIf(metric_key IN ('active_ai_members', 'cursor_active', 'cc_active', 'codex_active'), if(count(v) = 0, CAST(NULL, 'Nullable(Float64)'), toFloat64(0)), min(v)) AS company_p5,
    multiIf(metric_key IN ('active_ai_members', 'cursor_active', 'cc_active', 'codex_active'), if(count(v) = 0, CAST(NULL, 'Nullable(Float64)'), toFloat64(count())), max(v)) AS company_p95
FROM insight.ai_person_period
GROUP BY metric_key
;

CREATE OR REPLACE VIEW insight.ai_cost_person_period
(
    `person_id` Nullable(String),
    `org_unit_id` Nullable(String),
    `tool` String,
    `last_metric_date` Nullable(Date),
    `total_cost_cents` Nullable(Float64)
)
AS SELECT
    d.person_id AS person_id,
    p.org_unit_id AS org_unit_id,
    d.tool AS tool,
    d.last_metric_date AS last_metric_date,
    d.total_cost_cents AS total_cost_cents
FROM
(
    SELECT
        person_id,
        tool,
        max(metric_date) AS last_metric_date,
        sum(cost_cents) AS total_cost_cents
    FROM insight.ai_dev_tool_daily
    WHERE cost_cents IS NOT NULL
    GROUP BY
        person_id,
        tool
) AS d
LEFT JOIN insight.people AS p ON d.person_id = p.person_id
;

CREATE OR REPLACE VIEW insight.ai_dev_tool_daily
(
    `person_id` Nullable(String),
    `org_unit_id` Nullable(String),
    `metric_date` Nullable(Date),
    `tool` String,
    `source` String,
    `source_id` Nullable(String),
    `accepted_lines_added` Float64,
    `accepted_lines_removed` Float64,
    `tool_use_accepted` Nullable(Float64),
    `tool_use_offered` Nullable(Float64),
    `cost_cents` Nullable(Float64),
    `dev_agent_conversations` Nullable(Float64),
    `active_day` Float64
)
AS SELECT
    lower(c.email) AS person_id,
    p.org_unit_id AS org_unit_id,
    c.day AS metric_date,
    c.tool AS tool,
    c.source AS source,
    c.source_id AS source_id,
    sum(toFloat64(coalesce(c.lines_added, 0))) AS accepted_lines_added,
    sum(toFloat64(coalesce(c.lines_removed, 0))) AS accepted_lines_removed,
    if(countIf(c.tool_use_accepted IS NOT NULL) > 0, sumIf(toFloat64(c.tool_use_accepted), c.tool_use_accepted IS NOT NULL), CAST(NULL, 'Nullable(Float64)')) AS tool_use_accepted,
    if(countIf(c.tool_use_offered IS NOT NULL) > 0, sumIf(toFloat64(c.tool_use_offered), c.tool_use_offered IS NOT NULL), CAST(NULL, 'Nullable(Float64)')) AS tool_use_offered,
    if(countIf(c.cost_cents IS NOT NULL) > 0, sumIf(toFloat64(c.cost_cents), c.cost_cents IS NOT NULL), CAST(NULL, 'Nullable(Float64)')) AS cost_cents,
    if((c.tool IN ('claude_code', 'codex')) AND (countIf(c.session_count IS NOT NULL) > 0), sumIf(toFloat64(c.session_count), c.session_count IS NOT NULL), CAST(NULL, 'Nullable(Float64)')) AS dev_agent_conversations,
    toFloat64(1) AS active_day
FROM silver.class_ai_dev_usage AS c
LEFT JOIN insight.people AS p ON lower(c.email) = p.person_id
WHERE (c.email IS NOT NULL) AND (c.email != '')
GROUP BY
    person_id,
    org_unit_id,
    metric_date,
    tool,
    source,
    source_id
;

CREATE OR REPLACE VIEW insight.ai_dev_tool_person_period
(
    `person_id` Nullable(String),
    `org_unit_id` Nullable(String),
    `tool` String,
    `last_metric_date` Nullable(Date),
    `accepted_lines_added` Float64,
    `accepted_lines_removed` Float64,
    `tool_use_accepted` Nullable(Float64),
    `tool_use_offered` Nullable(Float64),
    `cost_cents` Nullable(Float64),
    `dev_agent_conversations` Nullable(Float64),
    `active_days` UInt64
)
AS SELECT
    d.person_id AS person_id,
    p.org_unit_id AS org_unit_id,
    d.tool AS tool,
    d.last_metric_date AS last_metric_date,
    d.accepted_lines_added AS accepted_lines_added,
    d.accepted_lines_removed AS accepted_lines_removed,
    d.tool_use_accepted AS tool_use_accepted,
    d.tool_use_offered AS tool_use_offered,
    d.cost_cents AS cost_cents,
    d.dev_agent_conversations AS dev_agent_conversations,
    d.active_days AS active_days
FROM
(
    SELECT
        person_id,
        tool,
        max(metric_date) AS last_metric_date,
        sum(accepted_lines_added) AS accepted_lines_added,
        sum(accepted_lines_removed) AS accepted_lines_removed,
        if(countIf(tool_use_accepted IS NOT NULL) > 0, sumIf(tool_use_accepted, tool_use_accepted IS NOT NULL), CAST(NULL, 'Nullable(Float64)')) AS tool_use_accepted,
        if(countIf(tool_use_offered IS NOT NULL) > 0, sumIf(tool_use_offered, tool_use_offered IS NOT NULL), CAST(NULL, 'Nullable(Float64)')) AS tool_use_offered,
        if(countIf(cost_cents IS NOT NULL) > 0, sumIf(cost_cents, cost_cents IS NOT NULL), CAST(NULL, 'Nullable(Float64)')) AS cost_cents,
        if(countIf(dev_agent_conversations IS NOT NULL) > 0, sumIf(dev_agent_conversations, dev_agent_conversations IS NOT NULL), CAST(NULL, 'Nullable(Float64)')) AS dev_agent_conversations,
        uniqExact(metric_date) AS active_days
    FROM insight.ai_dev_tool_daily
    GROUP BY
        person_id,
        tool
) AS d
LEFT JOIN insight.people AS p ON d.person_id = p.person_id
;

CREATE OR REPLACE VIEW insight.ai_dev_tool_trend
(
    `person_id` Nullable(String),
    `org_unit_id` Nullable(String),
    `metric_date` Nullable(Date),
    `tool` String,
    `accepted_lines_added` Float64
)
AS SELECT
    person_id,
    org_unit_id,
    metric_date,
    tool,
    sum(accepted_lines_added) AS accepted_lines_added
FROM insight.ai_dev_tool_daily
GROUP BY
    person_id,
    org_unit_id,
    metric_date,
    tool
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

CREATE OR REPLACE VIEW insight.ai_person_counter_daily
(
    `person_id` Nullable(String),
    `org_unit_id` Nullable(String),
    `metric_date` Nullable(Date),
    `ai_accepted_lines` Nullable(Float64),
    `ai_removed_lines` Nullable(Float64),
    `ai_active_days` Nullable(Float64),
    `ai_cost_cents` Nullable(Float64),
    `ai_accepted_edit_actions` Nullable(Float64),
    `ai_tool_acceptance_offered` Nullable(Float64),
    `ai_tool_acceptance_accepted` Nullable(Float64),
    `ai_assistant_messages` Nullable(Float64),
    `ai_assistant_actions` Nullable(Float64),
    `ai_dev_agent_conversations` Nullable(Float64),
    `ai_chat_assistant_conversations` Nullable(Float64)
)
AS SELECT
    d.person_id AS person_id,
    p.org_unit_id AS org_unit_id,
    d.metric_date AS metric_date,
    d.ai_accepted_lines AS ai_accepted_lines,
    d.ai_removed_lines AS ai_removed_lines,
    d.ai_active_days AS ai_active_days,
    d.ai_cost_cents AS ai_cost_cents,
    d.ai_accepted_edit_actions AS ai_accepted_edit_actions,
    d.ai_tool_acceptance_offered AS ai_tool_acceptance_offered,
    d.ai_tool_acceptance_accepted AS ai_tool_acceptance_accepted,
    d.ai_assistant_messages AS ai_assistant_messages,
    d.ai_assistant_actions AS ai_assistant_actions,
    d.ai_dev_agent_conversations AS ai_dev_agent_conversations,
    d.ai_chat_assistant_conversations AS ai_chat_assistant_conversations
FROM
(
    SELECT
        person_id,
        metric_date,
        if(countIf(ai_accepted_lines IS NOT NULL) > 0, sumIf(ai_accepted_lines, ai_accepted_lines IS NOT NULL), CAST(NULL, 'Nullable(Float64)')) AS ai_accepted_lines,
        if(countIf(ai_removed_lines IS NOT NULL) > 0, sumIf(ai_removed_lines, ai_removed_lines IS NOT NULL), CAST(NULL, 'Nullable(Float64)')) AS ai_removed_lines,
        if(countIf(ai_active_days IS NOT NULL) > 0, maxIf(ai_active_days, ai_active_days IS NOT NULL), CAST(NULL, 'Nullable(Float64)')) AS ai_active_days,
        if(countIf(ai_cost_cents IS NOT NULL) > 0, sumIf(ai_cost_cents, ai_cost_cents IS NOT NULL), CAST(NULL, 'Nullable(Float64)')) AS ai_cost_cents,
        if(countIf(ai_accepted_edit_actions IS NOT NULL) > 0, sumIf(ai_accepted_edit_actions, ai_accepted_edit_actions IS NOT NULL), CAST(NULL, 'Nullable(Float64)')) AS ai_accepted_edit_actions,
        if(countIf(ai_tool_acceptance_offered IS NOT NULL) > 0, sumIf(ai_tool_acceptance_offered, ai_tool_acceptance_offered IS NOT NULL), CAST(NULL, 'Nullable(Float64)')) AS ai_tool_acceptance_offered,
        if(countIf(ai_tool_acceptance_accepted IS NOT NULL) > 0, sumIf(ai_tool_acceptance_accepted, ai_tool_acceptance_accepted IS NOT NULL), CAST(NULL, 'Nullable(Float64)')) AS ai_tool_acceptance_accepted,
        if(countIf(ai_assistant_messages IS NOT NULL) > 0, sumIf(ai_assistant_messages, ai_assistant_messages IS NOT NULL), CAST(NULL, 'Nullable(Float64)')) AS ai_assistant_messages,
        if(countIf(ai_assistant_actions IS NOT NULL) > 0, sumIf(ai_assistant_actions, ai_assistant_actions IS NOT NULL), CAST(NULL, 'Nullable(Float64)')) AS ai_assistant_actions,
        if(countIf(ai_dev_agent_conversations IS NOT NULL) > 0, sumIf(ai_dev_agent_conversations, ai_dev_agent_conversations IS NOT NULL), CAST(NULL, 'Nullable(Float64)')) AS ai_dev_agent_conversations,
        if(countIf(ai_chat_assistant_conversations IS NOT NULL) > 0, sumIf(ai_chat_assistant_conversations, ai_chat_assistant_conversations IS NOT NULL), CAST(NULL, 'Nullable(Float64)')) AS ai_chat_assistant_conversations
    FROM
    (
        SELECT
            person_id,
            metric_date,
            toNullable(accepted_lines_added) AS ai_accepted_lines,
            toNullable(accepted_lines_removed) AS ai_removed_lines,
            toNullable(active_day) AS ai_active_days,
            cost_cents AS ai_cost_cents,
            tool_use_accepted AS ai_accepted_edit_actions,
            tool_use_offered AS ai_tool_acceptance_offered,
            tool_use_accepted AS ai_tool_acceptance_accepted,
            CAST(NULL, 'Nullable(Float64)') AS ai_assistant_messages,
            CAST(NULL, 'Nullable(Float64)') AS ai_assistant_actions,
            dev_agent_conversations AS ai_dev_agent_conversations,
            CAST(NULL, 'Nullable(Float64)') AS ai_chat_assistant_conversations
        FROM insight.ai_dev_tool_daily
        UNION ALL
        SELECT
            person_id,
            metric_date,
            CAST(NULL, 'Nullable(Float64)') AS ai_accepted_lines,
            CAST(NULL, 'Nullable(Float64)') AS ai_removed_lines,
            CAST(NULL, 'Nullable(Float64)') AS ai_active_days,
            CAST(NULL, 'Nullable(Float64)') AS ai_cost_cents,
            CAST(NULL, 'Nullable(Float64)') AS ai_accepted_edit_actions,
            CAST(NULL, 'Nullable(Float64)') AS ai_tool_acceptance_offered,
            CAST(NULL, 'Nullable(Float64)') AS ai_tool_acceptance_accepted,
            assistant_messages AS ai_assistant_messages,
            assistant_actions AS ai_assistant_actions,
            CAST(NULL, 'Nullable(Float64)') AS ai_dev_agent_conversations,
            chat_assistant_conversations AS ai_chat_assistant_conversations
        FROM insight.ai_assistant_tool_daily
    ) AS raw
    GROUP BY
        person_id,
        metric_date
) AS d
LEFT JOIN insight.people AS p ON d.person_id = p.person_id
;

CREATE OR REPLACE VIEW insight.ai_person_period
(
    `metric_key` String,
    `person_id` Nullable(String),
    `org_unit_id` Nullable(String),
    `metric_date` Nullable(Date),
    `v` Nullable(Float64)
)
AS SELECT
    metric_key,
    person_id,
    any(org_unit_id) AS org_unit_id,
    max(metric_date) AS metric_date,
    multiIf(metric_key IN ('chatgpt', 'cc_lines', 'cc_sessions', 'cursor_agents', 'cursor_lines', 'claude_web', 'cursor_completions', 'team_ai_loc', 'codex_lines', 'codex_sessions', 'cc_offered', 'cc_tool_accept', 'cc_cost', 'cc_overage', 'prs_total', 'prs_with_cc', 'cursor_offered', 'cursor_total_lines'), sum(metric_value), metric_key IN ('active_ai_members', 'cursor_active', 'cc_active', 'codex_active', 'chatgpt_active'), max(metric_value), avg(metric_value)) AS v
FROM insight.ai_bullet_rows
GROUP BY
    metric_key,
    person_id
;

CREATE OR REPLACE VIEW insight.code_quality_bullet_rows
(
    `person_id` String,
    `org_unit_id` Nullable(String),
    `metric_date` Date,
    `metric_key` String,
    `metric_value` Nullable(Float64)
)
AS SELECT
    j.person_id AS person_id,
    p.org_unit_id AS org_unit_id,
    j.metric_date AS metric_date,
    'bugs_fixed' AS metric_key,
    CAST(toFloat64(j.bugs_fixed), 'Nullable(Float64)') AS metric_value
FROM insight.jira_closed_tasks AS j
LEFT JOIN insight.people AS p ON j.person_id = p.person_id
;

CREATE OR REPLACE VIEW insight.code_quality_company_stats
(
    `metric_key` String,
    `company_value` Nullable(Float64),
    `company_median` Nullable(Float64),
    `company_p5` Nullable(Float64),
    `company_p95` Nullable(Float64)
)
AS SELECT
    metric_key,
    avg(v) AS company_value,
    quantileExact(0.5)(v) AS company_median,
    min(v) AS company_p5,
    max(v) AS company_p95
FROM insight.code_quality_person_period
GROUP BY metric_key
;

CREATE OR REPLACE VIEW insight.code_quality_person_period
(
    `metric_key` String,
    `person_id` String,
    `org_unit_id` Nullable(String),
    `metric_date` String,
    `v` Nullable(Float64)
)
AS SELECT
    metric_key,
    person_id,
    any(org_unit_id) AS org_unit_id,
    max(metric_date) AS metric_date,
    multiIf(metric_key IN ('bugs_fixed', 'prs_per_dev'), sum(metric_value), avg(metric_value)) AS v
FROM insight.code_quality_bullet_rows
GROUP BY
    metric_key,
    person_id
;

CREATE OR REPLACE VIEW insight.collab_bullet_rows
(
    `person_id` Nullable(String),
    `org_unit_id` Nullable(String),
    `metric_date` Nullable(Date),
    `metric_key` String,
    `metric_value` Float64
)
AS SELECT
    lower(e.email) AS person_id,
    p.org_unit_id AS org_unit_id,
    e.date AS metric_date,
    kv.1 AS metric_key,
    kv.2 AS metric_value
FROM silver.class_collab_email_activity AS e
LEFT JOIN insight.people AS p ON lower(e.email) = p.person_id
ARRAY JOIN [('m365_emails_sent', toFloat64(ifNull(e.sent_count, 0))), ('m365_emails_received', toFloat64(ifNull(e.received_count, 0))), ('m365_emails_read', toFloat64(ifNull(e.read_count, 0)))] AS kv
WHERE (e.data_source = 'insight_m365') AND (e.email IS NOT NULL) AND (e.email != '')
UNION ALL
SELECT
    pp.person_id AS person_id,
    pp.org_unit_id AS org_unit_id,
    pp.metric_date AS metric_date,
    kv.1 AS metric_key,
    kv.2 AS metric_value
FROM
(
    SELECT
        lower(ma.email) AS person_id,
        any(p.org_unit_id) AS org_unit_id,
        ma.date AS metric_date,
        sum(greatest(ifNull(ma.audio_duration_seconds, 0), ifNull(ma.video_duration_seconds, 0), ifNull(ma.screen_share_duration_seconds, 0))) / 3600. AS meeting_hours_v,
        sum(toFloat64(ifNull(ma.meetings_attended, 0))) AS meetings_count_v,
        sumIf(greatest(ifNull(ma.audio_duration_seconds, 0), ifNull(ma.video_duration_seconds, 0), ifNull(ma.screen_share_duration_seconds, 0)) / 3600., ma.data_source = 'insight_m365') AS teams_meeting_hours_v,
        sumIf(greatest(ifNull(ma.audio_duration_seconds, 0), ifNull(ma.video_duration_seconds, 0), ifNull(ma.screen_share_duration_seconds, 0)) / 3600., ma.data_source = 'insight_zoom') AS zoom_meeting_hours_v,
        sumIf(toFloat64(ifNull(ma.meetings_attended, 0)), ma.data_source = 'insight_m365') AS teams_meetings_v,
        sumIf(toFloat64(ifNull(ma.meetings_attended, 0)), ma.data_source = 'insight_zoom') AS zoom_meetings_v,
        if(sum((ifNull(ma.audio_duration_seconds, 0) + ifNull(ma.video_duration_seconds, 0)) + ifNull(ma.screen_share_duration_seconds, 0)) = 0, toFloat64(1), toFloat64(0)) AS meeting_free_v
    FROM silver.class_collab_meeting_activity AS ma
    FINAL
    LEFT JOIN insight.people AS p ON lower(ma.email) = p.person_id
    WHERE (ma.email IS NOT NULL) AND (ma.email != '')
    GROUP BY
        lower(ma.email),
        ma.date
) AS pp
ARRAY JOIN [('meeting_hours', pp.meeting_hours_v), ('meetings_count', pp.meetings_count_v), ('teams_meeting_hours', pp.teams_meeting_hours_v), ('zoom_meeting_hours', pp.zoom_meeting_hours_v), ('teams_meetings', pp.teams_meetings_v), ('zoom_meetings', pp.zoom_meetings_v), ('meeting_free', pp.meeting_free_v)] AS kv
UNION ALL
SELECT
    lower(c.email) AS person_id,
    p.org_unit_id AS org_unit_id,
    c.date AS metric_date,
    'm365_teams_chats' AS metric_key,
    toFloat64(ifNull(c.total_chat_messages, 0)) AS metric_value
FROM silver.class_collab_chat_activity AS c
LEFT JOIN insight.people AS p ON lower(c.email) = p.person_id
WHERE (c.data_source = 'insight_m365') AND (c.email IS NOT NULL) AND (c.email != '')
UNION ALL
SELECT
    lower(s.email) AS person_id,
    p.org_unit_id AS org_unit_id,
    s.date AS metric_date,
    kv.1 AS metric_key,
    kv.2 AS metric_value
FROM silver.class_collab_chat_activity AS s
LEFT JOIN insight.people AS p ON lower(s.email) = p.person_id
ARRAY JOIN [('slack_messages_sent', toFloat64(ifNull(s.total_chat_messages, 0))), ('slack_channel_posts', toFloat64(ifNull(s.channel_posts, 0))), ('slack_active_days', if(ifNull(s.total_chat_messages, 0) > 0, toFloat64(1), toFloat64(0)))] AS kv
WHERE (s.data_source = 'insight_slack') AND (s.email IS NOT NULL) AND (s.email != '')
UNION ALL
SELECT
    lower(z.email) AS person_id,
    p.org_unit_id AS org_unit_id,
    z.date AS metric_date,
    'zulip_messages_sent' AS metric_key,
    toFloat64(ifNull(z.total_chat_messages, 0)) AS metric_value
FROM silver.class_collab_chat_activity AS z
LEFT JOIN insight.people AS p ON lower(z.email) = p.person_id
WHERE (z.data_source = 'insight_zulip_proxy') AND (z.email IS NOT NULL) AND (z.email != '')
UNION ALL
SELECT
    lower(d.email) AS person_id,
    p.org_unit_id AS org_unit_id,
    d.date AS metric_date,
    kv.1 AS metric_key,
    kv.2 AS metric_value
FROM silver.class_collab_document_activity AS d
LEFT JOIN insight.people AS p ON lower(d.email) = p.person_id
ARRAY JOIN [('m365_files_shared_internal', toFloat64(ifNull(d.shared_internally_count, 0))), ('m365_files_shared_external', toFloat64(ifNull(d.shared_externally_count, 0))), ('m365_files_engaged', toFloat64(ifNull(d.viewed_or_edited_count, 0)))] AS kv
WHERE (d.data_source = 'insight_m365') AND (d.email IS NOT NULL) AND (d.email != '')
UNION ALL
SELECT
    person_id,
    any(p.org_unit_id) AS org_unit_id,
    metric_date,
    'm365_active_days' AS metric_key,
    if(sum(activity) > 0, toFloat64(1), toFloat64(0)) AS metric_value
FROM
(
    SELECT
        lower(email) AS person_id,
        date AS metric_date,
        toFloat64(ifNull(sent_count, 0)) AS activity
    FROM silver.class_collab_email_activity
    WHERE (data_source = 'insight_m365') AND (email IS NOT NULL) AND (email != '')
    UNION ALL
    SELECT
        lower(email),
        date,
        toFloat64(ifNull(total_chat_messages, 0))
    FROM silver.class_collab_chat_activity
    WHERE (data_source = 'insight_m365') AND (email IS NOT NULL) AND (email != '')
    UNION ALL
    SELECT
        lower(email),
        date,
        (toFloat64(ifNull(viewed_or_edited_count, 0)) + toFloat64(ifNull(shared_internally_count, 0))) + toFloat64(ifNull(shared_externally_count, 0))
    FROM silver.class_collab_document_activity
    WHERE (data_source = 'insight_m365') AND (email IS NOT NULL) AND (email != '')
) AS m365_daily
LEFT JOIN insight.people AS p ON p.person_id = m365_daily.person_id
GROUP BY
    person_id,
    metric_date
;

CREATE OR REPLACE VIEW insight.collab_company_stats
(
    `metric_key` String,
    `company_value` Nullable(Float64),
    `company_median` Nullable(Float64),
    `company_p5` Nullable(Float64),
    `company_p95` Nullable(Float64)
)
AS SELECT
    metric_key,
    avg(v) AS company_value,
    quantileExact(0.5)(v) AS company_median,
    min(v) AS company_p5,
    max(v) AS company_p95
FROM insight.collab_person_period
GROUP BY metric_key
;

CREATE OR REPLACE VIEW insight.collab_person_counter_daily
(
    `person_id` Nullable(String),
    `org_unit_id` Nullable(String),
    `metric_date` Nullable(Date),
    `messages_sent` Nullable(Float64),
    `channel_posts` Nullable(Float64)
)
AS SELECT
    d.person_id AS person_id,
    p.org_unit_id AS org_unit_id,
    d.metric_date AS metric_date,
    d.messages_sent AS messages_sent,
    d.channel_posts AS channel_posts
FROM
(
    SELECT
        person_id,
        metric_date,
        if(countIf(messages_sent IS NOT NULL) > 0, sumIf(messages_sent, messages_sent IS NOT NULL), CAST(NULL, 'Nullable(Float64)')) AS messages_sent,
        if(countIf(channel_posts IS NOT NULL) > 0, sumIf(channel_posts, channel_posts IS NOT NULL), CAST(NULL, 'Nullable(Float64)')) AS channel_posts
    FROM
    (
        SELECT
            lower(c.email) AS person_id,
            c.date AS metric_date,
            if(c.total_chat_messages IS NULL, CAST(NULL, 'Nullable(Float64)'), toFloat64(c.total_chat_messages)) AS messages_sent,
            if(c.channel_posts IS NULL, CAST(NULL, 'Nullable(Float64)'), toFloat64(c.channel_posts) + toFloat64(ifNull(c.channel_replies, 0))) AS channel_posts
        FROM silver.class_collab_chat_activity AS c
        FINAL
        WHERE (c.data_source = 'insight_m365') AND (c.email IS NOT NULL) AND (c.email != '')
        UNION ALL
        SELECT
            lower(s.email) AS person_id,
            s.date AS metric_date,
            if(s.total_chat_messages IS NULL, CAST(NULL, 'Nullable(Float64)'), toFloat64(s.total_chat_messages)) AS messages_sent,
            if(s.channel_posts IS NULL, CAST(NULL, 'Nullable(Float64)'), toFloat64(s.channel_posts)) AS channel_posts
        FROM silver.class_collab_chat_activity AS s
        FINAL
        WHERE (s.data_source = 'insight_slack') AND (s.email IS NOT NULL) AND (s.email != '')
        UNION ALL
        SELECT
            lower(z.email) AS person_id,
            z.date AS metric_date,
            if(z.total_chat_messages IS NULL, CAST(NULL, 'Nullable(Float64)'), toFloat64(z.total_chat_messages)) AS messages_sent,
            CAST(NULL, 'Nullable(Float64)') AS channel_posts
        FROM silver.class_collab_chat_activity AS z
        FINAL
        WHERE (z.data_source = 'insight_zulip_proxy') AND (z.email IS NOT NULL) AND (z.email != '')
    ) AS raw
    GROUP BY
        person_id,
        metric_date
) AS d
LEFT JOIN insight.people AS p ON d.person_id = p.person_id
;

CREATE OR REPLACE VIEW insight.collab_person_period
(
    `metric_key` String,
    `person_id` Nullable(String),
    `org_unit_id` Nullable(String),
    `metric_date` Nullable(String),
    `v` Nullable(Float64)
)
AS SELECT
    metric_key,
    person_id,
    any(org_unit_id) AS org_unit_id,
    max(metric_date) AS metric_date,
    multiIf(metric_key IN ('m365_emails_sent', 'zoom_calls', 'meeting_hours', 'm365_teams_messages', 'm365_files_shared', 'meeting_free', 'slack_thread_participation', 'slack_message_engagement'), sum(metric_value), avg(metric_value)) AS v
FROM insight.collab_bullet_rows
GROUP BY
    metric_key,
    person_id
;

CREATE OR REPLACE VIEW insight.commits_daily
(
    `person_id` String,
    `metric_date` Nullable(Date),
    `commits` UInt64
)
AS SELECT
    m.person_key AS person_id,
    m.week AS metric_date,
    toUInt64(m.commits) AS commits
FROM silver.mtr_git_person_weekly AS m
INNER JOIN insight.people AS p ON m.person_key = p.person_id
WHERE (p.status = 'Active') AND (m.week IS NOT NULL)
;

CREATE OR REPLACE VIEW insight.comms_daily
(
    `person_id` Nullable(String),
    `metric_date` Nullable(String),
    `emails_sent` Float64,
    `zoom_calls` Float64,
    `meeting_hours` Float64,
    `teams_messages` Float64,
    `teams_meetings` Float64,
    `files_shared` Float64
)
AS SELECT
    person_id,
    toString(metric_date) AS metric_date,
    sum(emails_sent) AS emails_sent,
    sum(zoom_calls) AS zoom_calls,
    sum(meeting_hours) AS meeting_hours,
    sum(teams_messages) AS teams_messages,
    sum(teams_meetings) AS teams_meetings,
    sum(files_shared) AS files_shared
FROM
(
    SELECT
        lower(person_key) AS person_id,
        date AS metric_date,
        toFloat64(coalesce(sent_count, 0)) AS emails_sent,
        toFloat64(0) AS zoom_calls,
        toFloat64(0) AS meeting_hours,
        toFloat64(0) AS teams_messages,
        toFloat64(0) AS teams_meetings,
        toFloat64(0) AS files_shared
    FROM silver.class_collab_email_activity
    WHERE data_source = 'insight_m365'
    UNION ALL
    SELECT
        lower(email),
        date,
        toFloat64(0),
        toFloat64(coalesce(calls_count, 0)),
        toFloat64(coalesce(audio_duration_seconds, 0)) / 3600.,
        toFloat64(0),
        toFloat64(0),
        toFloat64(0)
    FROM silver.class_collab_meeting_activity
    WHERE data_source = 'insight_zoom'
    UNION ALL
    SELECT
        lower(email),
        date,
        toFloat64(0),
        toFloat64(0),
        toFloat64(0),
        toFloat64(coalesce(total_chat_messages, 0)),
        toFloat64(0),
        toFloat64(0)
    FROM silver.class_collab_chat_activity
    WHERE data_source = 'insight_m365'
    UNION ALL
    SELECT
        lower(email),
        date,
        toFloat64(0),
        toFloat64(0),
        toFloat64(0),
        toFloat64(0),
        toFloat64(coalesce(meetings_attended, 0)),
        toFloat64(0)
    FROM silver.class_collab_meeting_activity
    WHERE data_source = 'insight_m365'
    UNION ALL
    SELECT
        lower(email),
        date,
        toFloat64(0),
        toFloat64(0),
        toFloat64(0),
        toFloat64(0),
        toFloat64(0),
        toFloat64(coalesce(shared_internally_count, 0)) + toFloat64(coalesce(shared_externally_count, 0))
    FROM silver.class_collab_document_activity
    WHERE data_source = 'insight_m365'
) AS sub
WHERE (person_id IS NOT NULL) AND (person_id != '')
GROUP BY
    person_id,
    metric_date
;

CREATE OR REPLACE VIEW insight.crm_bullet_rows
(
    `metric_date` Nullable(Date32),
    `person_id` Nullable(String),
    `org_unit_id` String,
    `metric_key` String,
    `metric_value` Nullable(Float64)
)
AS WITH
    deals_dedup AS
    (
        SELECT *
        FROM silver.class_crm_deals
        FINAL
    ),
    activities_dedup AS
    (
        SELECT *
        FROM silver.class_crm_activities
        FINAL
    ),
    owners AS
    (
        SELECT
            user_id,
            hs_user_id,
            lower(assumeNotNull(email)) AS person_id
        FROM silver.class_crm_users
        FINAL
        WHERE (email IS NOT NULL) AND (email != '')
    ),
    people AS
    (
        SELECT
            lower(assumeNotNull(email)) AS person_id,
            coalesce(department_name, 'Unknown') AS org_unit_id
        FROM silver.class_people
        FINAL
        WHERE (email IS NOT NULL) AND (email != '')
    ),
    opened_rows AS
    (
        SELECT
            toDate(d.created_at) AS metric_date,
            o.person_id AS person_id,
            coalesce(p.org_unit_id, 'Unknown') AS org_unit_id,
            'deals_opened' AS metric_key,
            toFloat64(1) AS metric_value
        FROM deals_dedup AS d
        INNER JOIN owners AS o ON o.user_id = d.owner_id
        LEFT JOIN people AS p ON p.person_id = o.person_id
        WHERE d.created_at IS NOT NULL
    ),
    closed_rows AS
    (
        SELECT
            d.close_date AS metric_date,
            o.person_id AS person_id,
            coalesce(p.org_unit_id, 'Unknown') AS org_unit_id,
            'deals_closed' AS metric_key,
            toFloat64(1) AS metric_value
        FROM deals_dedup AS d
        INNER JOIN owners AS o ON o.user_id = d.owner_id
        LEFT JOIN people AS p ON p.person_id = o.person_id
        WHERE (d.is_closed = 1) AND (d.close_date IS NOT NULL)
    ),
    won_rows AS
    (
        SELECT
            d.close_date AS metric_date,
            o.person_id AS person_id,
            coalesce(p.org_unit_id, 'Unknown') AS org_unit_id,
            'deals_won' AS metric_key,
            toFloat64(1) AS metric_value
        FROM deals_dedup AS d
        INNER JOIN owners AS o ON o.user_id = d.owner_id
        LEFT JOIN people AS p ON p.person_id = o.person_id
        WHERE (d.is_won = 1) AND (d.close_date IS NOT NULL)
    ),
    cycle_rows AS
    (
        SELECT
            d.close_date AS metric_date,
            o.person_id AS person_id,
            coalesce(p.org_unit_id, 'Unknown') AS org_unit_id,
            'cycle_days' AS metric_key,
            toFloat64(greatest(0, dateDiff('day', toDate(d.created_at), d.close_date))) AS metric_value
        FROM deals_dedup AS d
        INNER JOIN owners AS o ON o.user_id = d.owner_id
        LEFT JOIN people AS p ON p.person_id = o.person_id
        WHERE (d.is_won = 1) AND (d.close_date IS NOT NULL) AND (d.created_at IS NOT NULL)
    ),
    size_rows AS
    (
        SELECT
            d.close_date AS metric_date,
            o.person_id AS person_id,
            coalesce(p.org_unit_id, 'Unknown') AS org_unit_id,
            'deal_size' AS metric_key,
            toFloat64(coalesce(d.amount_home, 0)) AS metric_value
        FROM deals_dedup AS d
        INNER JOIN owners AS o ON o.user_id = d.owner_id
        LEFT JOIN people AS p ON p.person_id = o.person_id
        WHERE (d.is_won = 1) AND (d.close_date IS NOT NULL) AND (d.amount_home IS NOT NULL)
    ),
    activity_rows AS
    (
        SELECT
            toDate(a.timestamp) AS metric_date,
            if(a.activity_type = 'call', nullIf(by_owner.person_id, ''), nullIf(by_user.person_id, '')) AS person_id,
            coalesce(p.org_unit_id, 'Unknown') AS org_unit_id,
            multiIf(a.activity_type = 'call', 'calls', a.activity_type = 'email', 'emails', a.activity_type = 'meeting', 'meetings', a.activity_type = 'task', 'tasks', a.activity_type) AS metric_key,
            toFloat64(1) AS metric_value
        FROM activities_dedup AS a
        LEFT JOIN owners AS by_user ON by_user.hs_user_id = a.created_by_user_id
        LEFT JOIN owners AS by_owner ON by_owner.user_id = a.owner_id
        LEFT JOIN people AS p ON p.person_id = if(a.activity_type = 'call', nullIf(by_owner.person_id, ''), nullIf(by_user.person_id, ''))
        WHERE (a.timestamp IS NOT NULL) AND (if(a.activity_type = 'call', nullIf(by_owner.person_id, ''), nullIf(by_user.person_id, '')) IS NOT NULL)
    )
SELECT *
FROM opened_rows
UNION ALL
WITH
    deals_dedup AS
    (
        SELECT *
        FROM silver.class_crm_deals
        FINAL
    ),
    activities_dedup AS
    (
        SELECT *
        FROM silver.class_crm_activities
        FINAL
    ),
    owners AS
    (
        SELECT
            user_id,
            hs_user_id,
            lower(assumeNotNull(email)) AS person_id
        FROM silver.class_crm_users
        FINAL
        WHERE (email IS NOT NULL) AND (email != '')
    ),
    people AS
    (
        SELECT
            lower(assumeNotNull(email)) AS person_id,
            coalesce(department_name, 'Unknown') AS org_unit_id
        FROM silver.class_people
        FINAL
        WHERE (email IS NOT NULL) AND (email != '')
    ),
    opened_rows AS
    (
        SELECT
            toDate(d.created_at) AS metric_date,
            o.person_id AS person_id,
            coalesce(p.org_unit_id, 'Unknown') AS org_unit_id,
            'deals_opened' AS metric_key,
            toFloat64(1) AS metric_value
        FROM deals_dedup AS d
        INNER JOIN owners AS o ON o.user_id = d.owner_id
        LEFT JOIN people AS p ON p.person_id = o.person_id
        WHERE d.created_at IS NOT NULL
    ),
    closed_rows AS
    (
        SELECT
            d.close_date AS metric_date,
            o.person_id AS person_id,
            coalesce(p.org_unit_id, 'Unknown') AS org_unit_id,
            'deals_closed' AS metric_key,
            toFloat64(1) AS metric_value
        FROM deals_dedup AS d
        INNER JOIN owners AS o ON o.user_id = d.owner_id
        LEFT JOIN people AS p ON p.person_id = o.person_id
        WHERE (d.is_closed = 1) AND (d.close_date IS NOT NULL)
    ),
    won_rows AS
    (
        SELECT
            d.close_date AS metric_date,
            o.person_id AS person_id,
            coalesce(p.org_unit_id, 'Unknown') AS org_unit_id,
            'deals_won' AS metric_key,
            toFloat64(1) AS metric_value
        FROM deals_dedup AS d
        INNER JOIN owners AS o ON o.user_id = d.owner_id
        LEFT JOIN people AS p ON p.person_id = o.person_id
        WHERE (d.is_won = 1) AND (d.close_date IS NOT NULL)
    ),
    cycle_rows AS
    (
        SELECT
            d.close_date AS metric_date,
            o.person_id AS person_id,
            coalesce(p.org_unit_id, 'Unknown') AS org_unit_id,
            'cycle_days' AS metric_key,
            toFloat64(greatest(0, dateDiff('day', toDate(d.created_at), d.close_date))) AS metric_value
        FROM deals_dedup AS d
        INNER JOIN owners AS o ON o.user_id = d.owner_id
        LEFT JOIN people AS p ON p.person_id = o.person_id
        WHERE (d.is_won = 1) AND (d.close_date IS NOT NULL) AND (d.created_at IS NOT NULL)
    ),
    size_rows AS
    (
        SELECT
            d.close_date AS metric_date,
            o.person_id AS person_id,
            coalesce(p.org_unit_id, 'Unknown') AS org_unit_id,
            'deal_size' AS metric_key,
            toFloat64(coalesce(d.amount_home, 0)) AS metric_value
        FROM deals_dedup AS d
        INNER JOIN owners AS o ON o.user_id = d.owner_id
        LEFT JOIN people AS p ON p.person_id = o.person_id
        WHERE (d.is_won = 1) AND (d.close_date IS NOT NULL) AND (d.amount_home IS NOT NULL)
    ),
    activity_rows AS
    (
        SELECT
            toDate(a.timestamp) AS metric_date,
            if(a.activity_type = 'call', nullIf(by_owner.person_id, ''), nullIf(by_user.person_id, '')) AS person_id,
            coalesce(p.org_unit_id, 'Unknown') AS org_unit_id,
            multiIf(a.activity_type = 'call', 'calls', a.activity_type = 'email', 'emails', a.activity_type = 'meeting', 'meetings', a.activity_type = 'task', 'tasks', a.activity_type) AS metric_key,
            toFloat64(1) AS metric_value
        FROM activities_dedup AS a
        LEFT JOIN owners AS by_user ON by_user.hs_user_id = a.created_by_user_id
        LEFT JOIN owners AS by_owner ON by_owner.user_id = a.owner_id
        LEFT JOIN people AS p ON p.person_id = if(a.activity_type = 'call', nullIf(by_owner.person_id, ''), nullIf(by_user.person_id, ''))
        WHERE (a.timestamp IS NOT NULL) AND (if(a.activity_type = 'call', nullIf(by_owner.person_id, ''), nullIf(by_user.person_id, '')) IS NOT NULL)
    )
SELECT *
FROM closed_rows
UNION ALL
WITH
    deals_dedup AS
    (
        SELECT *
        FROM silver.class_crm_deals
        FINAL
    ),
    activities_dedup AS
    (
        SELECT *
        FROM silver.class_crm_activities
        FINAL
    ),
    owners AS
    (
        SELECT
            user_id,
            hs_user_id,
            lower(assumeNotNull(email)) AS person_id
        FROM silver.class_crm_users
        FINAL
        WHERE (email IS NOT NULL) AND (email != '')
    ),
    people AS
    (
        SELECT
            lower(assumeNotNull(email)) AS person_id,
            coalesce(department_name, 'Unknown') AS org_unit_id
        FROM silver.class_people
        FINAL
        WHERE (email IS NOT NULL) AND (email != '')
    ),
    opened_rows AS
    (
        SELECT
            toDate(d.created_at) AS metric_date,
            o.person_id AS person_id,
            coalesce(p.org_unit_id, 'Unknown') AS org_unit_id,
            'deals_opened' AS metric_key,
            toFloat64(1) AS metric_value
        FROM deals_dedup AS d
        INNER JOIN owners AS o ON o.user_id = d.owner_id
        LEFT JOIN people AS p ON p.person_id = o.person_id
        WHERE d.created_at IS NOT NULL
    ),
    closed_rows AS
    (
        SELECT
            d.close_date AS metric_date,
            o.person_id AS person_id,
            coalesce(p.org_unit_id, 'Unknown') AS org_unit_id,
            'deals_closed' AS metric_key,
            toFloat64(1) AS metric_value
        FROM deals_dedup AS d
        INNER JOIN owners AS o ON o.user_id = d.owner_id
        LEFT JOIN people AS p ON p.person_id = o.person_id
        WHERE (d.is_closed = 1) AND (d.close_date IS NOT NULL)
    ),
    won_rows AS
    (
        SELECT
            d.close_date AS metric_date,
            o.person_id AS person_id,
            coalesce(p.org_unit_id, 'Unknown') AS org_unit_id,
            'deals_won' AS metric_key,
            toFloat64(1) AS metric_value
        FROM deals_dedup AS d
        INNER JOIN owners AS o ON o.user_id = d.owner_id
        LEFT JOIN people AS p ON p.person_id = o.person_id
        WHERE (d.is_won = 1) AND (d.close_date IS NOT NULL)
    ),
    cycle_rows AS
    (
        SELECT
            d.close_date AS metric_date,
            o.person_id AS person_id,
            coalesce(p.org_unit_id, 'Unknown') AS org_unit_id,
            'cycle_days' AS metric_key,
            toFloat64(greatest(0, dateDiff('day', toDate(d.created_at), d.close_date))) AS metric_value
        FROM deals_dedup AS d
        INNER JOIN owners AS o ON o.user_id = d.owner_id
        LEFT JOIN people AS p ON p.person_id = o.person_id
        WHERE (d.is_won = 1) AND (d.close_date IS NOT NULL) AND (d.created_at IS NOT NULL)
    ),
    size_rows AS
    (
        SELECT
            d.close_date AS metric_date,
            o.person_id AS person_id,
            coalesce(p.org_unit_id, 'Unknown') AS org_unit_id,
            'deal_size' AS metric_key,
            toFloat64(coalesce(d.amount_home, 0)) AS metric_value
        FROM deals_dedup AS d
        INNER JOIN owners AS o ON o.user_id = d.owner_id
        LEFT JOIN people AS p ON p.person_id = o.person_id
        WHERE (d.is_won = 1) AND (d.close_date IS NOT NULL) AND (d.amount_home IS NOT NULL)
    ),
    activity_rows AS
    (
        SELECT
            toDate(a.timestamp) AS metric_date,
            if(a.activity_type = 'call', nullIf(by_owner.person_id, ''), nullIf(by_user.person_id, '')) AS person_id,
            coalesce(p.org_unit_id, 'Unknown') AS org_unit_id,
            multiIf(a.activity_type = 'call', 'calls', a.activity_type = 'email', 'emails', a.activity_type = 'meeting', 'meetings', a.activity_type = 'task', 'tasks', a.activity_type) AS metric_key,
            toFloat64(1) AS metric_value
        FROM activities_dedup AS a
        LEFT JOIN owners AS by_user ON by_user.hs_user_id = a.created_by_user_id
        LEFT JOIN owners AS by_owner ON by_owner.user_id = a.owner_id
        LEFT JOIN people AS p ON p.person_id = if(a.activity_type = 'call', nullIf(by_owner.person_id, ''), nullIf(by_user.person_id, ''))
        WHERE (a.timestamp IS NOT NULL) AND (if(a.activity_type = 'call', nullIf(by_owner.person_id, ''), nullIf(by_user.person_id, '')) IS NOT NULL)
    )
SELECT *
FROM won_rows
UNION ALL
WITH
    deals_dedup AS
    (
        SELECT *
        FROM silver.class_crm_deals
        FINAL
    ),
    activities_dedup AS
    (
        SELECT *
        FROM silver.class_crm_activities
        FINAL
    ),
    owners AS
    (
        SELECT
            user_id,
            hs_user_id,
            lower(assumeNotNull(email)) AS person_id
        FROM silver.class_crm_users
        FINAL
        WHERE (email IS NOT NULL) AND (email != '')
    ),
    people AS
    (
        SELECT
            lower(assumeNotNull(email)) AS person_id,
            coalesce(department_name, 'Unknown') AS org_unit_id
        FROM silver.class_people
        FINAL
        WHERE (email IS NOT NULL) AND (email != '')
    ),
    opened_rows AS
    (
        SELECT
            toDate(d.created_at) AS metric_date,
            o.person_id AS person_id,
            coalesce(p.org_unit_id, 'Unknown') AS org_unit_id,
            'deals_opened' AS metric_key,
            toFloat64(1) AS metric_value
        FROM deals_dedup AS d
        INNER JOIN owners AS o ON o.user_id = d.owner_id
        LEFT JOIN people AS p ON p.person_id = o.person_id
        WHERE d.created_at IS NOT NULL
    ),
    closed_rows AS
    (
        SELECT
            d.close_date AS metric_date,
            o.person_id AS person_id,
            coalesce(p.org_unit_id, 'Unknown') AS org_unit_id,
            'deals_closed' AS metric_key,
            toFloat64(1) AS metric_value
        FROM deals_dedup AS d
        INNER JOIN owners AS o ON o.user_id = d.owner_id
        LEFT JOIN people AS p ON p.person_id = o.person_id
        WHERE (d.is_closed = 1) AND (d.close_date IS NOT NULL)
    ),
    won_rows AS
    (
        SELECT
            d.close_date AS metric_date,
            o.person_id AS person_id,
            coalesce(p.org_unit_id, 'Unknown') AS org_unit_id,
            'deals_won' AS metric_key,
            toFloat64(1) AS metric_value
        FROM deals_dedup AS d
        INNER JOIN owners AS o ON o.user_id = d.owner_id
        LEFT JOIN people AS p ON p.person_id = o.person_id
        WHERE (d.is_won = 1) AND (d.close_date IS NOT NULL)
    ),
    cycle_rows AS
    (
        SELECT
            d.close_date AS metric_date,
            o.person_id AS person_id,
            coalesce(p.org_unit_id, 'Unknown') AS org_unit_id,
            'cycle_days' AS metric_key,
            toFloat64(greatest(0, dateDiff('day', toDate(d.created_at), d.close_date))) AS metric_value
        FROM deals_dedup AS d
        INNER JOIN owners AS o ON o.user_id = d.owner_id
        LEFT JOIN people AS p ON p.person_id = o.person_id
        WHERE (d.is_won = 1) AND (d.close_date IS NOT NULL) AND (d.created_at IS NOT NULL)
    ),
    size_rows AS
    (
        SELECT
            d.close_date AS metric_date,
            o.person_id AS person_id,
            coalesce(p.org_unit_id, 'Unknown') AS org_unit_id,
            'deal_size' AS metric_key,
            toFloat64(coalesce(d.amount_home, 0)) AS metric_value
        FROM deals_dedup AS d
        INNER JOIN owners AS o ON o.user_id = d.owner_id
        LEFT JOIN people AS p ON p.person_id = o.person_id
        WHERE (d.is_won = 1) AND (d.close_date IS NOT NULL) AND (d.amount_home IS NOT NULL)
    ),
    activity_rows AS
    (
        SELECT
            toDate(a.timestamp) AS metric_date,
            if(a.activity_type = 'call', nullIf(by_owner.person_id, ''), nullIf(by_user.person_id, '')) AS person_id,
            coalesce(p.org_unit_id, 'Unknown') AS org_unit_id,
            multiIf(a.activity_type = 'call', 'calls', a.activity_type = 'email', 'emails', a.activity_type = 'meeting', 'meetings', a.activity_type = 'task', 'tasks', a.activity_type) AS metric_key,
            toFloat64(1) AS metric_value
        FROM activities_dedup AS a
        LEFT JOIN owners AS by_user ON by_user.hs_user_id = a.created_by_user_id
        LEFT JOIN owners AS by_owner ON by_owner.user_id = a.owner_id
        LEFT JOIN people AS p ON p.person_id = if(a.activity_type = 'call', nullIf(by_owner.person_id, ''), nullIf(by_user.person_id, ''))
        WHERE (a.timestamp IS NOT NULL) AND (if(a.activity_type = 'call', nullIf(by_owner.person_id, ''), nullIf(by_user.person_id, '')) IS NOT NULL)
    )
SELECT *
FROM cycle_rows
UNION ALL
WITH
    deals_dedup AS
    (
        SELECT *
        FROM silver.class_crm_deals
        FINAL
    ),
    activities_dedup AS
    (
        SELECT *
        FROM silver.class_crm_activities
        FINAL
    ),
    owners AS
    (
        SELECT
            user_id,
            hs_user_id,
            lower(assumeNotNull(email)) AS person_id
        FROM silver.class_crm_users
        FINAL
        WHERE (email IS NOT NULL) AND (email != '')
    ),
    people AS
    (
        SELECT
            lower(assumeNotNull(email)) AS person_id,
            coalesce(department_name, 'Unknown') AS org_unit_id
        FROM silver.class_people
        FINAL
        WHERE (email IS NOT NULL) AND (email != '')
    ),
    opened_rows AS
    (
        SELECT
            toDate(d.created_at) AS metric_date,
            o.person_id AS person_id,
            coalesce(p.org_unit_id, 'Unknown') AS org_unit_id,
            'deals_opened' AS metric_key,
            toFloat64(1) AS metric_value
        FROM deals_dedup AS d
        INNER JOIN owners AS o ON o.user_id = d.owner_id
        LEFT JOIN people AS p ON p.person_id = o.person_id
        WHERE d.created_at IS NOT NULL
    ),
    closed_rows AS
    (
        SELECT
            d.close_date AS metric_date,
            o.person_id AS person_id,
            coalesce(p.org_unit_id, 'Unknown') AS org_unit_id,
            'deals_closed' AS metric_key,
            toFloat64(1) AS metric_value
        FROM deals_dedup AS d
        INNER JOIN owners AS o ON o.user_id = d.owner_id
        LEFT JOIN people AS p ON p.person_id = o.person_id
        WHERE (d.is_closed = 1) AND (d.close_date IS NOT NULL)
    ),
    won_rows AS
    (
        SELECT
            d.close_date AS metric_date,
            o.person_id AS person_id,
            coalesce(p.org_unit_id, 'Unknown') AS org_unit_id,
            'deals_won' AS metric_key,
            toFloat64(1) AS metric_value
        FROM deals_dedup AS d
        INNER JOIN owners AS o ON o.user_id = d.owner_id
        LEFT JOIN people AS p ON p.person_id = o.person_id
        WHERE (d.is_won = 1) AND (d.close_date IS NOT NULL)
    ),
    cycle_rows AS
    (
        SELECT
            d.close_date AS metric_date,
            o.person_id AS person_id,
            coalesce(p.org_unit_id, 'Unknown') AS org_unit_id,
            'cycle_days' AS metric_key,
            toFloat64(greatest(0, dateDiff('day', toDate(d.created_at), d.close_date))) AS metric_value
        FROM deals_dedup AS d
        INNER JOIN owners AS o ON o.user_id = d.owner_id
        LEFT JOIN people AS p ON p.person_id = o.person_id
        WHERE (d.is_won = 1) AND (d.close_date IS NOT NULL) AND (d.created_at IS NOT NULL)
    ),
    size_rows AS
    (
        SELECT
            d.close_date AS metric_date,
            o.person_id AS person_id,
            coalesce(p.org_unit_id, 'Unknown') AS org_unit_id,
            'deal_size' AS metric_key,
            toFloat64(coalesce(d.amount_home, 0)) AS metric_value
        FROM deals_dedup AS d
        INNER JOIN owners AS o ON o.user_id = d.owner_id
        LEFT JOIN people AS p ON p.person_id = o.person_id
        WHERE (d.is_won = 1) AND (d.close_date IS NOT NULL) AND (d.amount_home IS NOT NULL)
    ),
    activity_rows AS
    (
        SELECT
            toDate(a.timestamp) AS metric_date,
            if(a.activity_type = 'call', nullIf(by_owner.person_id, ''), nullIf(by_user.person_id, '')) AS person_id,
            coalesce(p.org_unit_id, 'Unknown') AS org_unit_id,
            multiIf(a.activity_type = 'call', 'calls', a.activity_type = 'email', 'emails', a.activity_type = 'meeting', 'meetings', a.activity_type = 'task', 'tasks', a.activity_type) AS metric_key,
            toFloat64(1) AS metric_value
        FROM activities_dedup AS a
        LEFT JOIN owners AS by_user ON by_user.hs_user_id = a.created_by_user_id
        LEFT JOIN owners AS by_owner ON by_owner.user_id = a.owner_id
        LEFT JOIN people AS p ON p.person_id = if(a.activity_type = 'call', nullIf(by_owner.person_id, ''), nullIf(by_user.person_id, ''))
        WHERE (a.timestamp IS NOT NULL) AND (if(a.activity_type = 'call', nullIf(by_owner.person_id, ''), nullIf(by_user.person_id, '')) IS NOT NULL)
    )
SELECT *
FROM size_rows
UNION ALL
WITH
    deals_dedup AS
    (
        SELECT *
        FROM silver.class_crm_deals
        FINAL
    ),
    activities_dedup AS
    (
        SELECT *
        FROM silver.class_crm_activities
        FINAL
    ),
    owners AS
    (
        SELECT
            user_id,
            hs_user_id,
            lower(assumeNotNull(email)) AS person_id
        FROM silver.class_crm_users
        FINAL
        WHERE (email IS NOT NULL) AND (email != '')
    ),
    people AS
    (
        SELECT
            lower(assumeNotNull(email)) AS person_id,
            coalesce(department_name, 'Unknown') AS org_unit_id
        FROM silver.class_people
        FINAL
        WHERE (email IS NOT NULL) AND (email != '')
    ),
    opened_rows AS
    (
        SELECT
            toDate(d.created_at) AS metric_date,
            o.person_id AS person_id,
            coalesce(p.org_unit_id, 'Unknown') AS org_unit_id,
            'deals_opened' AS metric_key,
            toFloat64(1) AS metric_value
        FROM deals_dedup AS d
        INNER JOIN owners AS o ON o.user_id = d.owner_id
        LEFT JOIN people AS p ON p.person_id = o.person_id
        WHERE d.created_at IS NOT NULL
    ),
    closed_rows AS
    (
        SELECT
            d.close_date AS metric_date,
            o.person_id AS person_id,
            coalesce(p.org_unit_id, 'Unknown') AS org_unit_id,
            'deals_closed' AS metric_key,
            toFloat64(1) AS metric_value
        FROM deals_dedup AS d
        INNER JOIN owners AS o ON o.user_id = d.owner_id
        LEFT JOIN people AS p ON p.person_id = o.person_id
        WHERE (d.is_closed = 1) AND (d.close_date IS NOT NULL)
    ),
    won_rows AS
    (
        SELECT
            d.close_date AS metric_date,
            o.person_id AS person_id,
            coalesce(p.org_unit_id, 'Unknown') AS org_unit_id,
            'deals_won' AS metric_key,
            toFloat64(1) AS metric_value
        FROM deals_dedup AS d
        INNER JOIN owners AS o ON o.user_id = d.owner_id
        LEFT JOIN people AS p ON p.person_id = o.person_id
        WHERE (d.is_won = 1) AND (d.close_date IS NOT NULL)
    ),
    cycle_rows AS
    (
        SELECT
            d.close_date AS metric_date,
            o.person_id AS person_id,
            coalesce(p.org_unit_id, 'Unknown') AS org_unit_id,
            'cycle_days' AS metric_key,
            toFloat64(greatest(0, dateDiff('day', toDate(d.created_at), d.close_date))) AS metric_value
        FROM deals_dedup AS d
        INNER JOIN owners AS o ON o.user_id = d.owner_id
        LEFT JOIN people AS p ON p.person_id = o.person_id
        WHERE (d.is_won = 1) AND (d.close_date IS NOT NULL) AND (d.created_at IS NOT NULL)
    ),
    size_rows AS
    (
        SELECT
            d.close_date AS metric_date,
            o.person_id AS person_id,
            coalesce(p.org_unit_id, 'Unknown') AS org_unit_id,
            'deal_size' AS metric_key,
            toFloat64(coalesce(d.amount_home, 0)) AS metric_value
        FROM deals_dedup AS d
        INNER JOIN owners AS o ON o.user_id = d.owner_id
        LEFT JOIN people AS p ON p.person_id = o.person_id
        WHERE (d.is_won = 1) AND (d.close_date IS NOT NULL) AND (d.amount_home IS NOT NULL)
    ),
    activity_rows AS
    (
        SELECT
            toDate(a.timestamp) AS metric_date,
            if(a.activity_type = 'call', nullIf(by_owner.person_id, ''), nullIf(by_user.person_id, '')) AS person_id,
            coalesce(p.org_unit_id, 'Unknown') AS org_unit_id,
            multiIf(a.activity_type = 'call', 'calls', a.activity_type = 'email', 'emails', a.activity_type = 'meeting', 'meetings', a.activity_type = 'task', 'tasks', a.activity_type) AS metric_key,
            toFloat64(1) AS metric_value
        FROM activities_dedup AS a
        LEFT JOIN owners AS by_user ON by_user.hs_user_id = a.created_by_user_id
        LEFT JOIN owners AS by_owner ON by_owner.user_id = a.owner_id
        LEFT JOIN people AS p ON p.person_id = if(a.activity_type = 'call', nullIf(by_owner.person_id, ''), nullIf(by_user.person_id, ''))
        WHERE (a.timestamp IS NOT NULL) AND (if(a.activity_type = 'call', nullIf(by_owner.person_id, ''), nullIf(by_user.person_id, '')) IS NOT NULL)
    )
SELECT *
FROM activity_rows
;

CREATE OR REPLACE VIEW insight.crm_chart_flow
(
    `person_id` String,
    `org_unit_id` String,
    `date_bucket` Nullable(String),
    `metric_date` Nullable(String),
    `opened` UInt64,
    `closed` UInt64,
    `won` UInt64
)
AS WITH
    deals_dedup AS
    (
        SELECT *
        FROM silver.class_crm_deals
        FINAL
    ),
    owners AS
    (
        SELECT
            user_id,
            lower(assumeNotNull(email)) AS person_id
        FROM silver.class_crm_users
        FINAL
        WHERE (email IS NOT NULL) AND (email != '')
    ),
    people AS
    (
        SELECT
            lower(assumeNotNull(email)) AS person_id,
            coalesce(department_name, 'Unknown') AS org_unit_id
        FROM silver.class_people
        FINAL
        WHERE (email IS NOT NULL) AND (email != '')
    ),
    opened_w AS
    (
        SELECT
            toMonday(toDate(d.created_at)) AS week_start,
            o.person_id AS person_id,
            count() AS opened,
            toUInt64(0) AS closed,
            toUInt64(0) AS won
        FROM deals_dedup AS d
        INNER JOIN owners AS o ON o.user_id = d.owner_id
        WHERE d.created_at IS NOT NULL
        GROUP BY
            week_start,
            person_id
    ),
    closed_w AS
    (
        SELECT
            toMonday(d.close_date) AS week_start,
            o.person_id AS person_id,
            toUInt64(0) AS opened,
            count() AS closed,
            countIf(d.is_won = 1) AS won
        FROM deals_dedup AS d
        INNER JOIN owners AS o ON o.user_id = d.owner_id
        WHERE (d.is_closed = 1) AND (d.close_date IS NOT NULL)
        GROUP BY
            week_start,
            person_id
    ),
    unioned AS
    (
        SELECT *
        FROM opened_w
        UNION ALL
        SELECT *
        FROM closed_w
    )
SELECT
    u.person_id AS person_id,
    coalesce(p.org_unit_id, 'Unknown') AS org_unit_id,
    formatDateTime(u.week_start, '%b %d') AS date_bucket,
    toString(u.week_start) AS metric_date,
    toUInt64(sum(u.opened)) AS opened,
    toUInt64(sum(u.closed)) AS closed,
    toUInt64(sum(u.won)) AS won
FROM unioned AS u
LEFT JOIN people AS p ON p.person_id = u.person_id
WHERE u.week_start IS NOT NULL
GROUP BY
    u.person_id,
    p.org_unit_id,
    u.week_start
;

CREATE OR REPLACE VIEW insight.crm_kpis
(
    `metric_date` Nullable(Date32),
    `person_id` Nullable(String),
    `org_unit_id` String,
    `org_unit_name` String,
    `deals_opened` UInt64,
    `deals_closed` UInt64,
    `deals_won` UInt64,
    `deals_value_closed` Float64,
    `comms_count` UInt64
)
AS WITH
    deals_dedup AS
    (
        SELECT *
        FROM silver.class_crm_deals
        FINAL
    ),
    activities_dedup AS
    (
        SELECT *
        FROM silver.class_crm_activities
        FINAL
    ),
    owners AS
    (
        SELECT
            user_id,
            hs_user_id,
            lower(assumeNotNull(email)) AS person_id
        FROM silver.class_crm_users
        FINAL
        WHERE (email IS NOT NULL) AND (email != '')
    ),
    people AS
    (
        SELECT
            lower(assumeNotNull(email)) AS person_id,
            coalesce(department_name, 'Unknown') AS org_unit_id
        FROM silver.class_people
        FINAL
        WHERE (email IS NOT NULL) AND (email != '')
    ),
    opened_by_day AS
    (
        SELECT
            toDate(d.created_at) AS metric_date,
            o.person_id AS person_id,
            count() AS deals_opened,
            toUInt64(0) AS deals_closed,
            toUInt64(0) AS deals_won,
            toFloat64(0) AS deals_value_closed,
            toUInt64(0) AS comms_count
        FROM deals_dedup AS d
        INNER JOIN owners AS o ON o.user_id = d.owner_id
        WHERE d.created_at IS NOT NULL
        GROUP BY
            metric_date,
            person_id
    ),
    closed_by_day AS
    (
        SELECT
            d.close_date AS metric_date,
            o.person_id AS person_id,
            toUInt64(0) AS deals_opened,
            count() AS deals_closed,
            countIf(d.is_won = 1) AS deals_won,
            sumIf(coalesce(d.amount_home, 0), d.is_won = 1) AS deals_value_closed,
            toUInt64(0) AS comms_count
        FROM deals_dedup AS d
        INNER JOIN owners AS o ON o.user_id = d.owner_id
        WHERE (d.is_closed = 1) AND (d.close_date IS NOT NULL)
        GROUP BY
            metric_date,
            person_id
    ),
    comms_by_day AS
    (
        SELECT
            toDate(a.timestamp) AS metric_date,
            if(a.activity_type = 'call', nullIf(by_owner.person_id, ''), nullIf(by_user.person_id, '')) AS person_id,
            toUInt64(0) AS deals_opened,
            toUInt64(0) AS deals_closed,
            toUInt64(0) AS deals_won,
            toFloat64(0) AS deals_value_closed,
            count() AS comms_count
        FROM activities_dedup AS a
        LEFT JOIN owners AS by_user ON by_user.hs_user_id = a.created_by_user_id
        LEFT JOIN owners AS by_owner ON by_owner.user_id = a.owner_id
        WHERE (a.timestamp IS NOT NULL) AND (if(a.activity_type = 'call', nullIf(by_owner.person_id, ''), nullIf(by_user.person_id, '')) IS NOT NULL)
        GROUP BY
            metric_date,
            person_id
    ),
    unioned AS
    (
        SELECT *
        FROM opened_by_day
        UNION ALL
        SELECT *
        FROM closed_by_day
        UNION ALL
        SELECT *
        FROM comms_by_day
    )
SELECT
    u.metric_date AS metric_date,
    u.person_id AS person_id,
    coalesce(p.org_unit_id, 'Unknown') AS org_unit_id,
    coalesce(p.org_unit_id, 'Unknown') AS org_unit_name,
    sum(u.deals_opened) AS deals_opened,
    sum(u.deals_closed) AS deals_closed,
    sum(u.deals_won) AS deals_won,
    sum(u.deals_value_closed) AS deals_value_closed,
    sum(u.comms_count) AS comms_count
FROM unioned AS u
LEFT JOIN people AS p ON p.person_id = u.person_id
WHERE u.metric_date IS NOT NULL
GROUP BY
    u.metric_date,
    u.person_id,
    p.org_unit_id
;

CREATE OR REPLACE VIEW insight.crm_pipeline_now
(
    `person_id` String,
    `org_unit_id` String,
    `pipeline_count` UInt64,
    `pipeline_value` Float64
)
AS WITH
    deals_dedup AS
    (
        SELECT *
        FROM silver.class_crm_deals
        FINAL
    ),
    owners AS
    (
        SELECT
            user_id,
            lower(assumeNotNull(email)) AS person_id
        FROM silver.class_crm_users
        FINAL
        WHERE (email IS NOT NULL) AND (email != '')
    ),
    people AS
    (
        SELECT
            lower(assumeNotNull(email)) AS person_id,
            coalesce(department_name, 'Unknown') AS org_unit_id
        FROM silver.class_people
        FINAL
        WHERE (email IS NOT NULL) AND (email != '')
    )
SELECT
    o.person_id AS person_id,
    coalesce(p.org_unit_id, 'Unknown') AS org_unit_id,
    countIf(d.is_closed = 0) AS pipeline_count,
    round(sumIf(coalesce(d.amount_home, 0), d.is_closed = 0)) AS pipeline_value
FROM deals_dedup AS d
INNER JOIN owners AS o ON o.user_id = d.owner_id
LEFT JOIN people AS p ON p.person_id = o.person_id
GROUP BY
    o.person_id,
    p.org_unit_id
;

CREATE OR REPLACE VIEW insight.email_daily
(
    `person_id` Nullable(String),
    `metric_date` Nullable(Date),
    `user_email` Nullable(String),
    `emails_sent` Nullable(Decimal(38, 9)),
    `source` String
)
AS SELECT
    lower(person_key) AS person_id,
    date AS metric_date,
    lower(person_key) AS user_email,
    sent_count AS emails_sent,
    data_source AS source
FROM staging.m365__collab_email_activity
;

CREATE OR REPLACE VIEW insight.exec_summary
(
    `org_unit_id` Nullable(String),
    `org_unit_name` Nullable(String),
    `headcount` UInt32,
    `tasks_closed` UInt64,
    `bugs_fixed` UInt64,
    `build_success_pct` Nullable(Float64),
    `focus_time_pct` Nullable(Float64),
    `ai_adoption_pct` Nullable(Float64),
    `ai_loc_share_pct` Nullable(Float64),
    `pr_cycle_time_h` Nullable(Float64),
    `metric_date` Nullable(String)
)
AS SELECT
    base.org_unit_id AS org_unit_id,
    base.org_unit_name AS org_unit_name,
    org.headcount AS headcount,
    ifNull(j.tasks_closed, 0) AS tasks_closed,
    ifNull(j.bugs_fixed, 0) AS bugs_fixed,
    CAST(NULL, 'Nullable(Float64)') AS build_success_pct,
    greatest(0, least(100, round(base.avg_focus_pct, 1))) AS focus_time_pct,
    if(ai.active_count IS NULL, CAST(NULL, 'Nullable(Float64)'), round((ai.active_count * 100.) / greatest(org.headcount, 1), 1)) AS ai_adoption_pct,
    if(ai.avg_ai_loc_share IS NULL, CAST(NULL, 'Nullable(Float64)'), round(ai.avg_ai_loc_share, 1)) AS ai_loc_share_pct,
    CAST(NULL, 'Nullable(Float64)') AS pr_cycle_time_h,
    base.metric_date AS metric_date
FROM
(
    SELECT
        pe.org_unit_id,
        any(pe.org_unit_name) AS org_unit_name,
        toString(f.day) AS metric_date,
        avg(f.focus_time_pct) AS avg_focus_pct
    FROM silver.class_focus_metrics AS f
    INNER JOIN insight.people AS pe ON (f.email = pe.person_id) AND (pe.status = 'Active')
    GROUP BY
        pe.org_unit_id,
        f.day
) AS base
INNER JOIN
(
    SELECT
        org_unit_id,
        toUInt32(count()) AS headcount
    FROM insight.people
    WHERE status = 'Active'
    GROUP BY org_unit_id
) AS org ON base.org_unit_id = org.org_unit_id
LEFT JOIN
(
    SELECT
        pe.org_unit_id,
        toString(j.metric_date) AS metric_date,
        sum(j.tasks_closed) AS tasks_closed,
        sum(j.bugs_fixed) AS bugs_fixed
    FROM insight.jira_closed_tasks AS j
    INNER JOIN insight.people AS pe ON (j.person_id = pe.person_id) AND (pe.status = 'Active')
    GROUP BY
        pe.org_unit_id,
        j.metric_date
) AS j ON (base.org_unit_id = j.org_unit_id) AND (base.metric_date = j.metric_date)
LEFT JOIN
(
    SELECT
        pe.org_unit_id,
        toString(c.day) AS metric_date,
        countDistinct(lower(c.email)) AS active_count,
        avg(if(toFloat64(coalesce(c.total_lines_added, 0)) > 0, (toFloat64(coalesce(c.lines_added, 0)) / toFloat64(c.total_lines_added)) * 100, CAST(NULL, 'Nullable(Float64)'))) AS avg_ai_loc_share
    FROM silver.class_ai_dev_usage AS c
    INNER JOIN insight.people AS pe ON (lower(c.email) = pe.person_id) AND (pe.status = 'Active')
    GROUP BY
        pe.org_unit_id,
        c.day
) AS ai ON (base.org_unit_id = ai.org_unit_id) AND (base.metric_date = ai.metric_date)
;

CREATE OR REPLACE VIEW insight.files_person_daily
(
    `person_id` Nullable(String),
    `metric_date` Nullable(Date),
    `files_shared` Float64
)
AS SELECT
    lower(email) AS person_id,
    date AS metric_date,
    toFloat64(sum(coalesce(shared_internally_count, 0))) + toFloat64(sum(coalesce(shared_externally_count, 0))) AS files_shared
FROM silver.class_collab_document_activity
WHERE (data_source = 'insight_m365') AND (email IS NOT NULL) AND (email != '')
GROUP BY
    lower(email),
    date
;

CREATE OR REPLACE VIEW insight.git_bullet_rows
(
    `person_id` String,
    `org_unit_id` Nullable(String),
    `metric_date` Date,
    `metric_key` String,
    `metric_value` Float64
)
AS SELECT
    pp.person_id AS person_id,
    pp.org_unit_id AS org_unit_id,
    pp.metric_date AS metric_date,
    kv.1 AS metric_key,
    kv.2 AS metric_value
FROM
(
    SELECT
        lower(c.author_email) AS person_id,
        any(p.org_unit_id) AS org_unit_id,
        assumeNotNull(toDate(c.date)) AS metric_date,
        toFloat64(countDistinct(c.commit_hash)) AS commits_v,
        toFloat64(sum(c.lines_added + c.lines_removed)) AS loc_v
    FROM silver.class_git_commits AS c
    FINAL
    LEFT JOIN insight.people AS p ON lower(c.author_email) = p.person_id
    WHERE (c.is_merge_commit = 0) AND (c.author_email != '') AND (c.date IS NOT NULL)
    GROUP BY
        lower(c.author_email),
        assumeNotNull(toDate(c.date))
) AS pp
ARRAY JOIN [('commits', pp.commits_v), ('loc', pp.loc_v)] AS kv
UNION ALL
SELECT
    lower(c.author_email) AS person_id,
    p.org_unit_id AS org_unit_id,
    assumeNotNull(toDate(c.date)) AS metric_date,
    'clean_loc' AS metric_key,
    toFloat64(sum(fc.lines_added)) AS metric_value
FROM silver.class_git_file_changes AS fc
FINAL
INNER JOIN silver.class_git_commits AS c
FINAL ON (c.tenant_id = fc.tenant_id) AND (c.commit_hash = fc.commit_hash) AND (c.project_key = fc.project_key) AND (c.repo_slug = fc.repo_slug)
LEFT JOIN insight.people AS p ON lower(c.author_email) = p.person_id
WHERE (c.is_merge_commit = 0) AND (c.author_email != '') AND (c.date IS NOT NULL) AND (multiIf(match(fc.file_path, '(?i)(\\.spec\\.|\\.test\\.|__tests__/|/tests?/)'), 'spec', match(fc.file_path, '(?i)(\\.lock$|package-lock\\.json|yarn\\.lock|poetry\\.lock|\\.ya?ml$|\\.toml$|\\.cfg$|\\.ini$)'), 'config', 'code') = 'code')
GROUP BY
    lower(c.author_email),
    p.org_unit_id,
    assumeNotNull(toDate(c.date))
UNION ALL
SELECT
    if(pr.author_email != '', lower(pr.author_email), lower(pr.author_name)) AS person_id,
    p.org_unit_id AS org_unit_id,
    assumeNotNull(toDate(pr.created_on)) AS metric_date,
    'prs_created' AS metric_key,
    toFloat64(count()) AS metric_value
FROM silver.class_git_pull_requests AS pr
FINAL
LEFT JOIN insight.people AS p ON if(pr.author_email != '', lower(pr.author_email), lower(pr.author_name)) = p.person_id
WHERE ((pr.author_email != '') OR (pr.author_name != '')) AND (pr.created_on IS NOT NULL)
GROUP BY
    person_id,
    p.org_unit_id,
    assumeNotNull(toDate(pr.created_on))
UNION ALL
SELECT
    if(pr.author_email != '', lower(pr.author_email), lower(pr.author_name)) AS person_id,
    p.org_unit_id AS org_unit_id,
    assumeNotNull(toDate(pr.closed_on)) AS metric_date,
    'prs_merged' AS metric_key,
    toFloat64(count()) AS metric_value
FROM silver.class_git_pull_requests AS pr
FINAL
LEFT JOIN insight.people AS p ON if(pr.author_email != '', lower(pr.author_email), lower(pr.author_name)) = p.person_id
WHERE ((pr.author_email != '') OR (pr.author_name != '')) AND (lower(pr.state) = 'merged') AND (pr.closed_on IS NOT NULL)
GROUP BY
    person_id,
    p.org_unit_id,
    assumeNotNull(toDate(pr.closed_on))
UNION ALL
SELECT
    if(pr.author_email != '', lower(pr.author_email), lower(pr.author_name)) AS person_id,
    p.org_unit_id AS org_unit_id,
    assumeNotNull(toDate(pr.created_on)) AS metric_date,
    'pr_size' AS metric_key,
    toFloat64(pr.lines_added + pr.lines_removed) AS metric_value
FROM silver.class_git_pull_requests AS pr
FINAL
LEFT JOIN insight.people AS p ON if(pr.author_email != '', lower(pr.author_email), lower(pr.author_name)) = p.person_id
WHERE ((pr.author_email != '') OR (pr.author_name != '')) AND (pr.created_on IS NOT NULL)
UNION ALL
SELECT
    if(pr.author_email != '', lower(pr.author_email), lower(pr.author_name)) AS person_id,
    p.org_unit_id AS org_unit_id,
    assumeNotNull(toDate(pr.closed_on)) AS metric_date,
    'pr_cycle_time_h' AS metric_key,
    assumeNotNull(toFloat64(dateDiff('second', pr.created_on, pr.closed_on) / 3600.)) AS metric_value
FROM silver.class_git_pull_requests AS pr
FINAL
LEFT JOIN insight.people AS p ON if(pr.author_email != '', lower(pr.author_email), lower(pr.author_name)) = p.person_id
WHERE ((pr.author_email != '') OR (pr.author_name != '')) AND (lower(pr.state) = 'merged') AND (pr.closed_on IS NOT NULL) AND (pr.created_on IS NOT NULL) AND (pr.closed_on >= pr.created_on)
;

CREATE OR REPLACE VIEW insight.ic_chart_delivery
(
    `person_id` String,
    `org_unit_id` Nullable(String),
    `date_bucket` Nullable(String),
    `metric_date` Nullable(String),
    `commits` UInt64,
    `prs_merged` Nullable(UInt64),
    `tasks_done` UInt64
)
AS WITH
    weekly_jira AS
    (
        SELECT
            person_id,
            toStartOfWeek(metric_date) AS week,
            sum(tasks_closed) AS tasks_done
        FROM insight.jira_closed_tasks
        GROUP BY
            person_id,
            week
    ),
    weeks_all AS
    (
        SELECT
            person_key AS person_id,
            week
        FROM silver.mtr_git_person_weekly
        WHERE (person_key != '') AND (week IS NOT NULL)
        UNION DISTINCT
        SELECT
            person_id,
            week
        FROM weekly_jira
    )
SELECT
    d.person_id AS person_id,
    p.org_unit_id AS org_unit_id,
    toString(d.week) AS date_bucket,
    toString(d.week) AS metric_date,
    toUInt64(ifNull(g.commits, 0)) AS commits,
    CAST(toUInt64(ifNull(g.prs_merged, 0)), 'Nullable(UInt64)') AS prs_merged,
    toUInt64(ifNull(j.tasks_done, 0)) AS tasks_done
FROM weeks_all AS d
LEFT JOIN insight.people AS p ON d.person_id = p.person_id
LEFT JOIN silver.mtr_git_person_weekly AS g ON (g.person_key = d.person_id) AND (g.week = d.week)
LEFT JOIN weekly_jira AS j ON (j.person_id = d.person_id) AND (j.week = d.week)
;

CREATE OR REPLACE VIEW insight.ic_chart_loc
(
    `person_id` Nullable(String),
    `org_unit_id` Nullable(String),
    `date_bucket` Nullable(String),
    `metric_date` Nullable(String),
    `code_loc` Float64,
    `spec_lines` Float64,
    `config_loc` Float64
)
AS SELECT
    g.person_key AS person_id,
    p.org_unit_id AS org_unit_id,
    toString(g.week) AS date_bucket,
    toString(g.week) AS metric_date,
    toFloat64(g.code_loc) AS code_loc,
    toFloat64(g.spec_lines) AS spec_lines,
    toFloat64(g.config_loc) AS config_loc
FROM silver.mtr_git_person_weekly AS g
LEFT JOIN insight.people AS p ON g.person_key = p.person_id
WHERE (g.person_key != '') AND (g.week IS NOT NULL)
;

CREATE OR REPLACE VIEW insight.ic_drill
(
    `person_id` String,
    `org_unit_id` String,
    `metric_date` String,
    `drill_id` String,
    `title` String,
    `source` String,
    `src_class` String,
    `value` String,
    `filter` String,
    `columns` Array(String),
    `rows` Array(String)
)
AS SELECT
    '' AS person_id,
    '' AS org_unit_id,
    '' AS metric_date,
    '' AS drill_id,
    '' AS title,
    '' AS source,
    '' AS src_class,
    '' AS value,
    '' AS filter,
    CAST([], 'Array(String)') AS columns,
    CAST([], 'Array(String)') AS rows
FROM system.one
WHERE 0
;

CREATE OR REPLACE VIEW insight.ic_histogram
(
    `person_id` Nullable(String),
    `org_unit_id` String,
    `metric_date` Nullable(Date),
    `metric_key` String,
    `bin` Nullable(Int64),
    `bin_end` Nullable(Int64),
    `count` UInt64
)
AS WITH
    all_rows AS
    (
        SELECT
            toString(person_id) AS person_id,
            toString(coalesce(org_unit_id, '')) AS org_unit_id,
            toDateOrNull(toString(metric_date)) AS metric_date,
            metric_key,
            toFloat64OrNull(toString(metric_value)) AS metric_value
        FROM insight.task_delivery_bullet_rows
        UNION ALL
        SELECT
            toString(person_id),
            toString(coalesce(org_unit_id, '')),
            toDateOrNull(toString(metric_date)),
            metric_key,
            toFloat64OrNull(toString(metric_value))
        FROM insight.code_quality_bullet_rows
        UNION ALL
        SELECT
            toString(person_id),
            toString(coalesce(org_unit_id, '')),
            toDateOrNull(toString(metric_date)),
            metric_key,
            toFloat64OrNull(toString(metric_value))
        FROM insight.ai_bullet_rows
        UNION ALL
        SELECT
            toString(person_id),
            toString(coalesce(org_unit_id, '')),
            toDateOrNull(toString(metric_date)),
            metric_key,
            toFloat64OrNull(toString(metric_value))
        FROM insight.collab_bullet_rows
        UNION ALL
        SELECT
            toString(person_id),
            toString(coalesce(org_unit_id, '')),
            toDateOrNull(toString(metric_date)),
            metric_key,
            toFloat64OrNull(toString(metric_value))
        FROM insight.git_bullet_rows
    ),
    classified AS
    (
        SELECT
            person_id,
            org_unit_id,
            metric_date,
            metric_key,
            metric_value,
            multiIf(metric_key IN ('task_dev_time', 'meeting_hours', 'teams_meeting_hours', 'zoom_meeting_hours'), 4, metric_key IN ('mean_time_to_resolution', 'pickup_time'), 1, metric_key IN ('estimation_accuracy', 'task_reopen_rate', 'due_date_compliance', 'flow_efficiency', 'merge_rate', 'build_success', 'cursor_acceptance', 'cc_tool_accept', 'ai_loc_share2', 'slack_dm_ratio'), 10, 0) AS step
        FROM all_rows
        WHERE metric_value IS NOT NULL
    )
SELECT
    person_id,
    org_unit_id,
    metric_date,
    metric_key,
    toInt64(floor(metric_value / step) * step) AS bin,
    toInt64((floor(metric_value / step) * step) + step) AS bin_end,
    count() AS count
FROM classified
WHERE step > 0
GROUP BY
    person_id,
    org_unit_id,
    metric_date,
    metric_key,
    bin,
    bin_end,
    step
;

CREATE OR REPLACE VIEW insight.ic_kpis
(
    `person_id` Nullable(String),
    `org_unit_id` Nullable(String),
    `metric_date` Nullable(String),
    `loc` Nullable(Float64),
    `ai_loc_share_pct` Nullable(Float64),
    `prs_merged` Nullable(Float64),
    `pr_cycle_time_h` Nullable(Float64),
    `focus_time_pct` Nullable(Float64),
    `tasks_closed` Nullable(Float64),
    `bugs_fixed` Nullable(Float64),
    `build_success_pct` Nullable(Float64),
    `ai_sessions` Nullable(Float64)
)
AS SELECT
    f.email AS person_id,
    p.org_unit_id AS org_unit_id,
    toString(f.day) AS metric_date,
    CAST(NULL, 'Nullable(Float64)') AS loc,
    round(ifNull(cur.ai_loc_share_pct, 0), 1) AS ai_loc_share_pct,
    CAST(NULL, 'Nullable(Float64)') AS prs_merged,
    CAST(NULL, 'Nullable(Float64)') AS pr_cycle_time_h,
    greatest(0, least(100, round(ifNull(f.focus_time_pct, 100), 1))) AS focus_time_pct,
    toFloat64(ifNull(j.tasks_closed, 0)) AS tasks_closed,
    toFloat64(ifNull(j.bugs_fixed, 0)) AS bugs_fixed,
    CAST(NULL, 'Nullable(Float64)') AS build_success_pct,
    toFloat64(ifNull(cur.ai_sessions, 0)) AS ai_sessions
FROM silver.class_focus_metrics AS f
LEFT JOIN insight.people AS p ON f.email = p.person_id
LEFT JOIN
(
    SELECT
        person_id,
        toString(metric_date) AS metric_date,
        sum(tasks_closed) AS tasks_closed,
        sum(bugs_fixed) AS bugs_fixed
    FROM insight.jira_closed_tasks
    GROUP BY
        person_id,
        metric_date
) AS j ON (f.email = j.person_id) AND (toString(f.day) = j.metric_date)
LEFT JOIN
(
    SELECT
        lower(email) AS person_id,
        toString(day) AS metric_date,
        if(toFloat64(coalesce(total_lines_added, 0)) > 0, round((toFloat64(coalesce(lines_added, 0)) / toFloat64(total_lines_added)) * 100, 1), CAST(NULL, 'Nullable(Float64)')) AS ai_loc_share_pct,
        toFloat64(coalesce(agent_sessions, 0)) + toFloat64(coalesce(chat_requests, 0)) AS ai_sessions
    FROM silver.class_ai_dev_usage
) AS cur ON (f.email = cur.person_id) AND (toString(f.day) = cur.metric_date)
UNION ALL
SELECT
    g.person_key AS person_id,
    p.org_unit_id AS org_unit_id,
    toString(g.week) AS metric_date,
    CAST(toFloat64(g.code_loc), 'Nullable(Float64)') AS loc,
    CAST(NULL, 'Nullable(Float64)') AS ai_loc_share_pct,
    CAST(toFloat64(g.prs_merged), 'Nullable(Float64)') AS prs_merged,
    CAST(NULL, 'Nullable(Float64)') AS pr_cycle_time_h,
    CAST(NULL, 'Nullable(Float64)') AS focus_time_pct,
    CAST(NULL, 'Nullable(Float64)') AS tasks_closed,
    CAST(NULL, 'Nullable(Float64)') AS bugs_fixed,
    CAST(NULL, 'Nullable(Float64)') AS build_success_pct,
    CAST(NULL, 'Nullable(Float64)') AS ai_sessions
FROM silver.mtr_git_person_weekly AS g
INNER JOIN insight.people AS p ON g.person_key = p.person_id
WHERE (p.status = 'Active') AND (g.week IS NOT NULL)
;

CREATE OR REPLACE VIEW insight.ic_section_trend
(
    `person_id` Nullable(String),
    `org_unit_id` String,
    `metric_date` Nullable(Date),
    `section_id` String,
    `series_key` String,
    `value` Nullable(Float64)
)
AS SELECT
    toString(person_id) AS person_id,
    toString(coalesce(org_unit_id, '')) AS org_unit_id,
    toDateOrNull(toString(metric_date)) AS metric_date,
    'task_delivery' AS section_id,
    'tasks_completed' AS series_key,
    sum(toFloat64OrNull(toString(metric_value))) AS value
FROM insight.task_delivery_bullet_rows
WHERE metric_key = 'tasks_completed'
GROUP BY
    person_id,
    org_unit_id,
    metric_date
UNION ALL
SELECT
    toString(person_id),
    toString(coalesce(org_unit_id, '')),
    toDateOrNull(toString(metric_date)),
    'git_output' AS section_id,
    metric_key AS series_key,
    sum(toFloat64OrNull(toString(metric_value)))
FROM insight.git_bullet_rows
WHERE metric_key IN ('commits', 'prs_merged')
GROUP BY
    person_id,
    org_unit_id,
    metric_date,
    metric_key
UNION ALL
SELECT
    toString(person_id),
    toString(coalesce(org_unit_id, '')),
    toDateOrNull(toString(metric_date)),
    'code_quality' AS section_id,
    metric_key AS series_key,
    sum(toFloat64OrNull(toString(metric_value)))
FROM insight.code_quality_bullet_rows
WHERE metric_key IN ('bugs_fixed', 'build_success', 'pr_cycle_time')
GROUP BY
    person_id,
    org_unit_id,
    metric_date,
    metric_key
UNION ALL
SELECT
    toString(person_id),
    toString(coalesce(org_unit_id, '')),
    toDateOrNull(toString(metric_date)),
    'ai_adoption' AS section_id,
    metric_key AS series_key,
    sum(toFloat64OrNull(toString(metric_value)))
FROM insight.ai_bullet_rows
WHERE metric_key IN ('cursor_lines', 'cc_lines')
GROUP BY
    person_id,
    org_unit_id,
    metric_date,
    metric_key
UNION ALL
SELECT
    toString(person_id),
    toString(coalesce(org_unit_id, '')),
    toDateOrNull(toString(metric_date)),
    'collaboration' AS section_id,
    'meeting_hours' AS series_key,
    sum(toFloat64OrNull(toString(metric_value)))
FROM insight.collab_bullet_rows
WHERE metric_key = 'meeting_hours'
GROUP BY
    person_id,
    org_unit_id,
    metric_date
UNION ALL
SELECT
    toString(person_id),
    toString(coalesce(org_unit_id, '')),
    toDateOrNull(toString(metric_date)),
    'collaboration' AS section_id,
    'total_messages' AS series_key,
    sum(toFloat64OrNull(toString(metric_value)))
FROM insight.collab_bullet_rows
WHERE metric_key IN ('slack_messages_sent', 'm365_emails_sent', 'm365_teams_chats')
GROUP BY
    person_id,
    org_unit_id,
    metric_date
;

CREATE OR REPLACE VIEW insight.ic_timeoff
(
    `person_id` String,
    `org_unit_id` String,
    `metric_date` String,
    `days` UInt32,
    `date_range` String,
    `bamboo_hr_url` String
)
AS SELECT
    '' AS person_id,
    '' AS org_unit_id,
    '' AS metric_date,
    toUInt32(0) AS days,
    '' AS date_range,
    '' AS bamboo_hr_url
FROM system.one
WHERE 0
;

CREATE OR REPLACE VIEW insight.jira_closed_tasks
(
    `person_id` String,
    `metric_date` Date,
    `tasks_closed` UInt64,
    `bugs_fixed` UInt64,
    `on_time_count` UInt64,
    `has_due_date_count` UInt64,
    `avg_time_spent` Nullable(Float64),
    `avg_time_estimate` Nullable(Float64)
)
AS SELECT
    coalesce(s.assignee_email, '') AS person_id,
    toDate(s.final_close_at) AS metric_date,
    toUInt64(count()) AS tasks_closed,
    toUInt64(countIf(s.issue_type = 'Bug')) AS bugs_fixed,
    toUInt64(countIf((s.due_date_str IS NOT NULL) AND (s.due_date_str != '') AND (toDate(s.final_close_at) <= toDate(parseDateTimeBestEffortOrNull(s.due_date_str))))) AS on_time_count,
    toUInt64(countIf((s.due_date_str IS NOT NULL) AND (s.due_date_str != ''))) AS has_due_date_count,
    avgIf(s.time_spent_seconds_field, ifNull(s.time_estimate_seconds, toFloat64(0)) > 0) AS avg_time_spent,
    avgIf(s.time_estimate_seconds, ifNull(s.time_estimate_seconds, toFloat64(0)) > 0) AS avg_time_estimate
FROM insight.task_issue_current_state AS s
WHERE (s.final_close_at IS NOT NULL) AND (s.assignee_email IS NOT NULL) AND (s.assignee_email != '') AND (s.status_category = 'done')
GROUP BY
    person_id,
    metric_date
;

CREATE OR REPLACE VIEW insight.jira_person_daily
(
    `person_id` Nullable(String),
    `metric_date` Nullable(Date),
    `issue_type` Nullable(String),
    `status_name` Nullable(String),
    `resolution` Nullable(String),
    `due_date` Nullable(String),
    `time_estimate_sec` Nullable(Float64),
    `time_spent_sec` Nullable(Float64),
    `id_readable` Nullable(String)
)
AS SELECT
    lower(JSONExtractString(latest_fields, 'assignee', 'emailAddress')) AS person_id,
    toDate(parseDateTimeBestEffortOrNull(latest_updated)) AS metric_date,
    JSONExtractString(latest_fields, 'issuetype', 'name') AS issue_type,
    JSONExtractString(latest_fields, 'status', 'name') AS status_name,
    JSONExtractString(latest_fields, 'resolution', 'name') AS resolution,
    latest_due_date AS due_date,
    JSONExtractFloat(latest_fields, 'timeoriginalestimate') AS time_estimate_sec,
    JSONExtractFloat(latest_fields, 'timespent') AS time_spent_sec,
    latest_id_readable AS id_readable
FROM
(
    SELECT
        unique_key,
        argMax(custom_fields_json, _airbyte_extracted_at) AS latest_fields,
        argMax(updated, _airbyte_extracted_at) AS latest_updated,
        argMax(due_date, _airbyte_extracted_at) AS latest_due_date,
        argMax(id_readable, _airbyte_extracted_at) AS latest_id_readable
    FROM bronze_jira.jira_issue
    WHERE unique_key IS NOT NULL
    GROUP BY unique_key
)
WHERE JSONExtractString(latest_fields, 'assignee', 'emailAddress') != ''
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

CREATE OR REPLACE VIEW insight.people
(
    `person_id` Nullable(String),
    `display_name` Nullable(String),
    `org_unit_id` Nullable(String),
    `org_unit_name` Nullable(String),
    `seniority` String,
    `job_title` Nullable(String),
    `status` Nullable(String),
    `supervisor_email` Nullable(String)
)
AS SELECT
    person_id,
    argMax(displayName, _airbyte_extracted_at) AS display_name,
    argMax(department, _airbyte_extracted_at) AS org_unit_id,
    argMax(department, _airbyte_extracted_at) AS org_unit_name,
    argMax(multiIf((jobTitle ILIKE '%senior%') OR (jobTitle ILIKE '%lead%') OR (jobTitle ILIKE '%principal%') OR (jobTitle ILIKE '%architect%') OR (jobTitle ILIKE '%director%') OR (jobTitle ILIKE '%head%'), 'Senior', (jobTitle ILIKE '%junior%') OR (jobTitle ILIKE '%intern%') OR (jobTitle ILIKE '%trainee%'), 'Junior', 'Mid'), _airbyte_extracted_at) AS seniority,
    argMax(jobTitle, _airbyte_extracted_at) AS job_title,
    argMax(status, _airbyte_extracted_at) AS status,
    lower(argMax(supervisorEmail, _airbyte_extracted_at)) AS supervisor_email
FROM bronze_bamboohr.employees
WHERE (workEmail IS NOT NULL) AND (workEmail != '')
GROUP BY lower(workEmail) AS person_id
;

CREATE OR REPLACE VIEW insight.support_bullet_rows
(
    `person_id` Nullable(String),
    `org_unit_id` Nullable(String),
    `metric_date` Nullable(Date),
    `metric_key` String,
    `metric_value` Float64
)
AS SELECT
    a.person_key AS person_id,
    p.org_unit_id AS org_unit_id,
    a.date AS metric_date,
    kv.1 AS metric_key,
    kv.2 AS metric_value
FROM silver.class_support_activity AS a
LEFT JOIN insight.people AS p ON a.person_key = p.person_id
ARRAY JOIN [('support_active', toFloat64(if((((ifNull(a.updates, 0) + ifNull(a.public_comments, 0)) + ifNull(a.private_comments, 0)) + ifNull(a.solved, 0)) > 0, 1, 0))), ('support_updates', toFloat64(ifNull(a.updates, 0))), ('support_public_comments', toFloat64(ifNull(a.public_comments, 0))), ('support_private_comments', toFloat64(ifNull(a.private_comments, 0))), ('support_solved', toFloat64(ifNull(a.solved, 0))), ('support_csat_good', toFloat64(ifNull(a.csat_good, 0))), ('support_csat_total', toFloat64(ifNull(a.csat_total, 0)))] AS kv
WHERE (a.person_key IS NOT NULL) AND (a.person_key != '')
;

CREATE OR REPLACE VIEW insight.support_company_stats
(
    `metric_key` String,
    `company_value` Float64,
    `company_median` Nullable(Float64),
    `company_p5` Nullable(Float64),
    `company_p95` Nullable(Float64)
)
AS SELECT
    metric_key,
    multiIf(metric_key IN ('support_active'), sum(v), avg(v)) AS company_value,
    multiIf(metric_key IN ('support_active'), if(count(v) = 0, CAST(NULL, 'Nullable(Float64)'), toFloat64(0)), quantileExact(0.5)(v)) AS company_median,
    multiIf(metric_key IN ('support_active'), if(count(v) = 0, CAST(NULL, 'Nullable(Float64)'), toFloat64(0)), min(v)) AS company_p5,
    multiIf(metric_key IN ('support_active'), if(count(v) = 0, CAST(NULL, 'Nullable(Float64)'), toFloat64(count())), max(v)) AS company_p95
FROM insight.support_person_period
GROUP BY metric_key
;

CREATE OR REPLACE VIEW insight.support_person_period
(
    `metric_key` String,
    `person_id` Nullable(String),
    `org_unit_id` Nullable(String),
    `metric_date` Nullable(Date),
    `v` Float64
)
AS SELECT
    metric_key,
    person_id,
    any(org_unit_id) AS org_unit_id,
    max(metric_date) AS metric_date,
    multiIf(metric_key IN ('support_active'), max(metric_value), sum(metric_value)) AS v
FROM insight.support_bullet_rows
GROUP BY
    metric_key,
    person_id
;

CREATE OR REPLACE VIEW insight.task_close_events_daily
(
    `assignee_email` Nullable(String),
    `event_date` Date,
    `close_count` UInt64
)
AS WITH transitions AS
    (
        SELECT
            insight_source_id,
            issue_id,
            interval_start AS event_at,
            status_category,
            lagInFrame(status_category) OVER (PARTITION BY insight_source_id, issue_id ORDER BY interval_start ASC) AS prev_category
        FROM insight.task_status_intervals
    )
SELECT
    s.assignee_email AS assignee_email,
    toDate(t.event_at) AS event_date,
    count() AS close_count
FROM transitions AS t
INNER JOIN insight.task_issue_current_state AS s ON (s.insight_source_id = t.insight_source_id) AND (s.issue_id = t.issue_id)
WHERE ((t.prev_category IS NULL) OR (t.prev_category != 'done')) AND (t.status_category = 'done') AND (s.assignee_email IS NOT NULL) AND (s.assignee_email != '')
GROUP BY
    assignee_email,
    event_date
;

CREATE OR REPLACE VIEW insight.task_delivery_bullet_rows
(
    `person_id` Nullable(String),
    `org_unit_id` Nullable(String),
    `metric_date` Date,
    `metric_key` String,
    `metric_value` Nullable(Float64)
)
AS SELECT
    j.person_id AS person_id,
    p.org_unit_id AS org_unit_id,
    j.metric_date AS metric_date,
    kv.1 AS metric_key,
    kv.2 AS metric_value
FROM insight.jira_closed_tasks AS j
LEFT JOIN insight.people AS p ON j.person_id = p.person_id
ARRAY JOIN [('tasks_completed', CAST(toFloat64(j.tasks_closed), 'Nullable(Float64)')), ('due_date_on_time', CAST(toFloat64(j.on_time_count), 'Nullable(Float64)')), ('due_date_with_due', CAST(toFloat64(j.has_due_date_count), 'Nullable(Float64)')), ('estimation_accuracy', if((ifNull(j.avg_time_spent, toFloat64(0)) > 0) AND (j.avg_time_estimate IS NOT NULL), CAST(round((j.avg_time_estimate / j.avg_time_spent) * 100, 1), 'Nullable(Float64)'), CAST(NULL, 'Nullable(Float64)'))), ('bugs_fixed', CAST(toFloat64(j.bugs_fixed), 'Nullable(Float64)'))] AS kv
UNION ALL
SELECT
    ip.assignee_email AS person_id,
    p.org_unit_id AS org_unit_id,
    ip.close_date AS metric_date,
    kv.1 AS metric_key,
    kv.2 AS metric_value
FROM insight.task_dev_seconds_per_issue AS ip
LEFT JOIN insight.people AS p ON ip.assignee_email = p.person_id
ARRAY JOIN [('task_dev_time', if((ip.dev_seconds IS NULL) OR (ip.dev_seconds = 0), CAST(NULL, 'Nullable(Float64)'), CAST(round(toFloat64(ip.dev_seconds) / 3600., 2), 'Nullable(Float64)'))), ('mean_time_to_resolution', if((ip.lead_seconds IS NULL) OR (ip.lead_seconds = 0), CAST(NULL, 'Nullable(Float64)'), CAST(round(toFloat64(ip.lead_seconds) / 86400., 2), 'Nullable(Float64)'))), ('flow_efficiency_num', if((ip.dev_seconds IS NULL) OR (ip.dev_seconds = 0) OR (ip.lead_seconds IS NULL) OR (ip.lead_seconds <= 0), CAST(NULL, 'Nullable(Float64)'), CAST(toFloat64(ip.dev_seconds), 'Nullable(Float64)'))), ('flow_efficiency_den', if((ip.dev_seconds IS NULL) OR (ip.dev_seconds = 0) OR (ip.lead_seconds IS NULL) OR (ip.lead_seconds <= 0), CAST(NULL, 'Nullable(Float64)'), CAST(toFloat64(ip.lead_seconds), 'Nullable(Float64)'))), ('pickup_time', if(ip.pickup_seconds IS NULL, CAST(NULL, 'Nullable(Float64)'), CAST(round(toFloat64(ip.pickup_seconds) / 86400., 2), 'Nullable(Float64)')))] AS kv
UNION ALL
SELECT
    c.assignee_email AS person_id,
    p.org_unit_id AS org_unit_id,
    c.event_date AS metric_date,
    'task_reopen_rate' AS metric_key,
    CAST(toFloat64(c.close_count), 'Nullable(Float64)') AS metric_value
FROM insight.task_close_events_daily AS c
LEFT JOIN insight.people AS p ON c.assignee_email = p.person_id
UNION ALL
SELECT
    r.assignee_email AS person_id,
    p.org_unit_id AS org_unit_id,
    r.event_date AS metric_date,
    'task_reopen_rate' AS metric_key,
    CAST(-toFloat64(r.reopen_count), 'Nullable(Float64)') AS metric_value
FROM insight.task_reopen_events_daily AS r
LEFT JOIN insight.people AS p ON r.assignee_email = p.person_id
UNION ALL
SELECT
    coalesce(w.author_email, ip.assignee_email) AS person_id,
    p.org_unit_id AS org_unit_id,
    coalesce(w.work_date, ip.day) AS metric_date,
    kv.1 AS metric_key,
    kv.2 AS metric_value
FROM insight.task_worklog_seconds_per_day AS w
FULL OUTER JOIN insight.task_in_progress_seconds_per_day AS ip ON (w.author_email = ip.assignee_email) AND (w.work_date = ip.day)
LEFT JOIN insight.people AS p ON p.person_id = coalesce(w.author_email, ip.assignee_email)
ARRAY JOIN [('worklog_seconds', if(ifNull(ip.in_progress_seconds, toFloat64(0)) > 0, CAST(toFloat64(ifNull(w.worklog_seconds, toFloat64(0))), 'Nullable(Float64)'), CAST(NULL, 'Nullable(Float64)'))), ('in_progress_seconds', if(ifNull(ip.in_progress_seconds, toFloat64(0)) > 0, CAST(toFloat64(ip.in_progress_seconds), 'Nullable(Float64)'), CAST(NULL, 'Nullable(Float64)')))] AS kv
UNION ALL
SELECT
    s.assignee_email AS person_id,
    p.org_unit_id AS org_unit_id,
    today() AS metric_date,
    'stale_in_progress' AS metric_key,
    CAST(toFloat64(count()), 'Nullable(Float64)') AS metric_value
FROM insight.task_issue_current_state AS s
LEFT JOIN insight.people AS p ON s.assignee_email = p.person_id
WHERE ((s.status_category IS NULL) OR (s.status_category != 'done')) AND (s.assignee_email IS NOT NULL) AND (s.assignee_email != '') AND (s.last_status_event_at IS NOT NULL) AND (dateDiff('day', s.last_status_event_at, now()) > 14)
GROUP BY
    s.assignee_email,
    p.org_unit_id
;

CREATE OR REPLACE VIEW insight.task_dev_seconds_per_issue
(
    `assignee_email` Nullable(String),
    `insight_source_id` String,
    `issue_id` String,
    `close_date` Date,
    `dev_seconds` Float64,
    `lead_seconds` Nullable(Float64),
    `pickup_seconds` Nullable(Float64)
)
AS SELECT
    s.assignee_email AS assignee_email,
    s.insight_source_id AS insight_source_id,
    s.issue_id AS issue_id,
    toDate(s.final_close_at) AS close_date,
    sum(i.duration_seconds) AS dev_seconds,
    if(any(s.created_at) IS NULL, CAST(NULL, 'Nullable(Float64)'), toFloat64(greatest(toInt64(0), dateDiff('second', any(s.created_at), any(s.final_close_at))))) AS lead_seconds,
    if((any(s.created_at) IS NULL) OR (min(i.interval_start) IS NULL), CAST(NULL, 'Nullable(Float64)'), toFloat64(greatest(toInt64(0), dateDiff('second', any(s.created_at), min(i.interval_start))))) AS pickup_seconds
FROM insight.task_issue_current_state AS s
LEFT JOIN insight.task_status_intervals AS i ON (i.insight_source_id = s.insight_source_id) AND (i.issue_id = s.issue_id) AND (i.status_category = 'in_progress')
WHERE (s.final_close_at IS NOT NULL) AND (s.assignee_email IS NOT NULL) AND (s.assignee_email != '')
GROUP BY
    s.assignee_email,
    s.insight_source_id,
    s.issue_id,
    close_date
;

CREATE OR REPLACE VIEW insight.task_in_progress_seconds_per_day
(
    `assignee_email` Nullable(String),
    `day` Date,
    `in_progress_seconds` Float64
)
AS WITH ip AS
    (
        SELECT
            s.assignee_email AS assignee_email,
            i.interval_start AS interval_start,
            i.interval_end AS interval_end
        FROM insight.task_status_intervals AS i
        INNER JOIN insight.task_issue_current_state AS s ON (s.insight_source_id = i.insight_source_id) AND (s.issue_id = i.issue_id)
        WHERE (i.status_category = 'in_progress') AND (s.assignee_email IS NOT NULL) AND (s.assignee_email != '')
    )
SELECT
    assignee_email,
    day,
    sum(toFloat64(greatest(toInt64(0), dateDiff('second', greatest(interval_start, toDateTime(day)), least(interval_end, toDateTime(day) + toIntervalDay(1)))))) AS in_progress_seconds
FROM ip
ARRAY JOIN arrayMap(d -> (toDate(interval_start) + toIntervalDay(d)), range(toUInt32(dateDiff('day', toDate(interval_start), toDate(interval_end)) + 1))) AS day
GROUP BY
    assignee_email,
    day
;

CREATE MATERIALIZED VIEW IF NOT EXISTS insight.task_issue_current_state
REFRESH EVERY 1 HOUR
(
    `insight_source_id` String,
    `data_source` String,
    `issue_id` String,
    `status_name` String,
    `status_id` String,
    `status_category` String,
    `assignee_account_id` String,
    `issue_type` String,
    `priority` String,
    `due_date_str` String,
    `time_estimate_seconds` Nullable(Float64),
    `time_spent_seconds_field` Nullable(Float64),
    `created_at` DateTime64(3),
    `final_close_at` DateTime64(3),
    `last_status_event_at` DateTime64(3),
    `assignee_email` Nullable(String),
    `org_unit_id` Nullable(String)
)
ENGINE = MergeTree
ORDER BY (insight_source_id, issue_id)
SETTINGS index_granularity = 8192, allow_nullable_key = 1
DEFINER = insight SQL SECURITY DEFINER
AS WITH
    issue_state AS
    (
        SELECT
            insight_source_id,
            data_source,
            issue_id,
            argMaxIf(value_displays[1], (event_at, _version), (field_id = 'status') AND (delta_action = 'set')) AS status_name,
            argMaxIf(value_ids[1], (event_at, _version), (field_id = 'status') AND (delta_action = 'set')) AS status_id,
            argMaxIf(value_ids[1], (event_at, _version), (field_id = 'assignee') AND (delta_action = 'set')) AS assignee_account_id,
            argMaxIf(value_displays[1], (event_at, _version), (field_id = 'issuetype') AND (delta_action = 'set')) AS issue_type,
            argMaxIf(value_displays[1], (event_at, _version), (field_id = 'priority') AND (delta_action = 'set')) AS priority,
            argMaxIf(value_displays[1], (event_at, _version), (field_id = 'duedate') AND (delta_action = 'set')) AS due_date_str,
            toFloat64OrNull(argMaxIf(value_displays[1], (event_at, _version), (field_id = 'timeoriginalestimate') AND (delta_action = 'set'))) AS time_estimate_seconds,
            toFloat64OrNull(argMaxIf(value_displays[1], (event_at, _version), (field_id = 'timespent') AND (delta_action = 'set'))) AS time_spent_seconds_field,
            minIf(event_at, event_kind = 'synthetic_initial') AS created_at,
            maxIf(event_at, (field_id = 'status') AND (delta_action = 'set')) AS last_status_event_at
        FROM silver.class_task_field_history
        WHERE (field_id IN ('status', 'assignee', 'issuetype', 'priority', 'duedate', 'timeoriginalestimate', 'timespent')) OR (event_kind = 'synthetic_initial')
        GROUP BY
            insight_source_id,
            data_source,
            issue_id
    ),
    status_cat AS
    (
        SELECT
            fh.insight_source_id AS insight_source_id,
            fh.issue_id AS issue_id,
            maxIf(fh.event_at, st.status_category = 'done') AS final_close_at
        FROM silver.class_task_field_history AS fh
        LEFT JOIN silver.class_task_statuses AS st
        FINAL ON (st.insight_source_id = fh.insight_source_id) AND (st.status_id = (fh.value_ids[1]))
        WHERE (fh.field_id = 'status') AND (fh.delta_action = 'set')
        GROUP BY
            fh.insight_source_id,
            fh.issue_id
    )
SELECT
    s.insight_source_id AS insight_source_id,
    s.data_source AS data_source,
    s.issue_id AS issue_id,
    s.status_name AS status_name,
    s.status_id AS status_id,
    cur.status_category AS status_category,
    s.assignee_account_id AS assignee_account_id,
    s.issue_type AS issue_type,
    s.priority AS priority,
    s.due_date_str AS due_date_str,
    s.time_estimate_seconds AS time_estimate_seconds,
    s.time_spent_seconds_field AS time_spent_seconds_field,
    s.created_at AS created_at,
    sc.final_close_at AS final_close_at,
    s.last_status_event_at AS last_status_event_at,
    lower(u.email) AS assignee_email,
    p.org_unit_id AS org_unit_id
FROM issue_state AS s
LEFT JOIN status_cat AS sc ON (sc.insight_source_id = s.insight_source_id) AND (sc.issue_id = s.issue_id)
LEFT JOIN silver.class_task_statuses AS cur
FINAL ON (cur.insight_source_id = s.insight_source_id) AND (cur.status_id = s.status_id)
LEFT JOIN silver.class_task_users AS u
FINAL ON (u.insight_source_id = s.insight_source_id) AND (u.user_id = s.assignee_account_id)
LEFT JOIN insight.people AS p ON p.person_id = lower(u.email)
;

CREATE OR REPLACE VIEW insight.task_reopen_events_daily
(
    `assignee_email` Nullable(String),
    `event_date` Date,
    `reopen_count` UInt64
)
AS WITH transitions AS
    (
        SELECT
            insight_source_id,
            issue_id,
            interval_start AS event_at,
            status_category,
            lagInFrame(status_category) OVER (PARTITION BY insight_source_id, issue_id ORDER BY interval_start ASC) AS prev_category
        FROM insight.task_status_intervals
    )
SELECT
    s.assignee_email AS assignee_email,
    toDate(t.event_at) AS event_date,
    count() AS reopen_count
FROM transitions AS t
INNER JOIN insight.task_issue_current_state AS s ON (s.insight_source_id = t.insight_source_id) AND (s.issue_id = t.issue_id)
WHERE (t.prev_category = 'done') AND ((t.status_category != 'done') OR (t.status_category IS NULL)) AND (s.assignee_email IS NOT NULL) AND (s.assignee_email != '')
GROUP BY
    assignee_email,
    event_date
;

CREATE MATERIALIZED VIEW IF NOT EXISTS insight.task_status_intervals
REFRESH EVERY 1 HOUR
(
    `insight_source_id` String,
    `issue_id` String,
    `interval_start` DateTime64(3),
    `interval_end` DateTime64(3),
    `status_id` String,
    `status_name` String,
    `status_category` String,
    `duration_seconds` Float64
)
ENGINE = MergeTree
ORDER BY (insight_source_id, issue_id, interval_start)
SETTINGS index_granularity = 8192, allow_nullable_key = 1
DEFINER = insight SQL SECURITY DEFINER
AS WITH events AS
    (
        SELECT
            insight_source_id,
            issue_id,
            arraySort(x -> (x.1), groupArray((event_at, value_ids[1], value_displays[1]))) AS evs
        FROM silver.class_task_field_history
        FINAL
        WHERE (field_id = 'status') AND (delta_action = 'set')
        GROUP BY
            insight_source_id,
            issue_id
    )
SELECT
    iv.insight_source_id AS insight_source_id,
    iv.issue_id AS issue_id,
    iv.interval_start AS interval_start,
    iv.interval_end AS interval_end,
    iv.status_id AS status_id,
    iv.status_name AS status_name,
    st.status_category AS status_category,
    iv.duration_seconds AS duration_seconds
FROM
(
    SELECT
        e.insight_source_id AS insight_source_id,
        e.issue_id AS issue_id,
        arrayJoin(arrayMap(i -> ((e.evs[i]).1, if(i = length(e.evs), ifNull(s.final_close_at, now()), (e.evs[i + 1]).1), (e.evs[i]).2, (e.evs[i]).3), range(1, length(e.evs) + 1))) AS row,
        row.1 AS interval_start,
        row.2 AS interval_end,
        row.3 AS status_id,
        row.4 AS status_name,
        toFloat64(greatest(toInt64(0), dateDiff('second', row.1, row.2))) AS duration_seconds,
        s.created_at AS issue_created_at
    FROM events AS e
    LEFT JOIN insight.task_issue_current_state AS s ON (s.insight_source_id = e.insight_source_id) AND (s.issue_id = e.issue_id)
) AS iv
LEFT JOIN silver.class_task_statuses AS st
FINAL ON (st.insight_source_id = iv.insight_source_id) AND (st.status_id = iv.status_id)
WHERE (iv.interval_start >= ifNull(iv.issue_created_at, toDateTime('1970-01-02'))) AND (iv.interval_end >= iv.interval_start) AND (iv.interval_end <= (now() + toIntervalDay(1)))
;

CREATE OR REPLACE VIEW insight.task_worklog_seconds_per_day
(
    `author_email` Nullable(String),
    `work_date` Nullable(Date),
    `worklog_seconds` Float64
)
AS SELECT
    lower(u.email) AS author_email,
    toDate(w.work_date) AS work_date,
    sum(ifNull(w.duration_seconds, toFloat64(0))) AS worklog_seconds
FROM silver.class_task_worklogs AS w
FINAL
INNER JOIN silver.class_task_users AS u
FINAL ON (u.insight_source_id = w.insight_source_id) AND (u.user_id = w.author_id)
WHERE (u.email IS NOT NULL) AND (u.email != '')
GROUP BY
    author_email,
    work_date
;

CREATE OR REPLACE VIEW insight.team_member
(
    `person_id` Nullable(String),
    `display_name` Nullable(String),
    `seniority` String,
    `org_unit_id` Nullable(String),
    `tasks_closed` Float64,
    `bugs_fixed` Float64,
    `dev_time_h` Nullable(Float64),
    `prs_merged` Nullable(Float64),
    `build_success_pct` Nullable(Float64),
    `focus_time_pct` Nullable(Float64),
    `ai_tools` Array(String),
    `ai_loc_share_pct` Nullable(Float64),
    `metric_date` Nullable(String)
)
AS SELECT
    p.person_id AS person_id,
    p.display_name AS display_name,
    p.seniority AS seniority,
    p.org_unit_id AS org_unit_id,
    toFloat64(ifNull(j.tasks_closed, 0)) AS tasks_closed,
    toFloat64(ifNull(j.bugs_fixed, 0)) AS bugs_fixed,
    if(f.dev_time_h IS NULL, CAST(NULL, 'Nullable(Float64)'), CAST(greatest(0, round(f.dev_time_h, 1)), 'Nullable(Float64)')) AS dev_time_h,
    CAST(NULL, 'Nullable(Float64)') AS prs_merged,
    CAST(NULL, 'Nullable(Float64)') AS build_success_pct,
    if(f.focus_time_pct IS NULL, CAST(NULL, 'Nullable(Float64)'), CAST(greatest(0, least(100, round(f.focus_time_pct, 1))), 'Nullable(Float64)')) AS focus_time_pct,
    if(cur.email IS NOT NULL, ['Cursor'], CAST([], 'Array(String)')) AS ai_tools,
    if(cur.email IS NULL, CAST(NULL, 'Nullable(Float64)'), CAST(round(cur.ai_loc_share_pct, 1), 'Nullable(Float64)')) AS ai_loc_share_pct,
    f.metric_date AS metric_date
FROM insight.people AS p
INNER JOIN
(
    SELECT
        email,
        toString(day) AS metric_date,
        focus_time_pct,
        dev_time_h
    FROM silver.class_focus_metrics
) AS f ON p.person_id = f.email
LEFT JOIN
(
    SELECT
        person_id,
        toString(metric_date) AS metric_date,
        sum(tasks_closed) AS tasks_closed,
        sum(bugs_fixed) AS bugs_fixed
    FROM insight.jira_closed_tasks
    GROUP BY
        person_id,
        metric_date
) AS j ON (p.person_id = j.person_id) AND (f.metric_date = j.metric_date)
LEFT JOIN
(
    SELECT
        lower(email) AS email,
        toString(day) AS metric_date,
        if(toFloat64(coalesce(total_lines_added, 0)) > 0, round((toFloat64(coalesce(lines_added, 0)) / toFloat64(total_lines_added)) * 100, 1), CAST(NULL, 'Nullable(Float64)')) AS ai_loc_share_pct
    FROM silver.class_ai_dev_usage
) AS cur ON (p.person_id = cur.email) AND (f.metric_date = cur.metric_date)
WHERE p.status = 'Active'
;

CREATE OR REPLACE VIEW insight.teams_person_daily
(
    `person_id` Nullable(String),
    `metric_date` Nullable(Date),
    `teams_messages` Float64,
    `teams_meetings` Float64,
    `teams_calls` Float64
)
AS SELECT
    lower(coalesce(c.email, m.email)) AS person_id,
    coalesce(c.date, m.date) AS metric_date,
    toFloat64(coalesce(c.total_chat_messages, 0)) AS teams_messages,
    toFloat64(coalesce(m.meetings_attended, 0)) AS teams_meetings,
    toFloat64(coalesce(m.calls_count, 0)) AS teams_calls
FROM
(
    SELECT
        email,
        date,
        total_chat_messages
    FROM silver.class_collab_chat_activity
    WHERE data_source = 'insight_m365'
) AS c
FULL OUTER JOIN
(
    SELECT
        email,
        date,
        meetings_attended,
        calls_count
    FROM silver.class_collab_meeting_activity
    WHERE data_source = 'insight_m365'
) AS m ON (lower(c.email) = lower(m.email)) AND (c.date = m.date)
;

CREATE OR REPLACE VIEW insight.wiki_bullet_rows
(
    `person_id` Nullable(String),
    `org_unit_id` Nullable(String),
    `metric_date` Nullable(Date),
    `metric_key` String,
    `metric_value` Float64
)
AS SELECT
    coalesce(lower(pg.author_email), pg.author_id) AS person_id,
    p.org_unit_id AS org_unit_id,
    toDate(pg.created_at) AS metric_date,
    kv.1 AS metric_key,
    kv.2 AS metric_value
FROM
(
    SELECT *
    FROM silver.class_wiki_pages
    FINAL
) AS pg
LEFT JOIN insight.people AS p ON coalesce(lower(pg.author_email), pg.author_id) = p.person_id
ARRAY JOIN [('wiki_pages_created', toFloat64(1)), ('wiki_edits', toFloat64(greatest(toInt64(pg.version_count) - 1, 0))), ('wiki_active_authors', toFloat64(1))] AS kv
WHERE (pg.author_id IS NOT NULL) AND (pg.author_id != '') AND (pg.created_at IS NOT NULL)
UNION ALL
SELECT
    coalesce(lower(pg.author_email), pg.author_id) AS person_id,
    p.org_unit_id AS org_unit_id,
    e.day AS metric_date,
    'wiki_comments' AS metric_key,
    toFloat64(e.total_comments) AS metric_value
FROM
(
    SELECT *
    FROM silver.class_wiki_engagement
    FINAL
) AS e
INNER JOIN
(
    SELECT *
    FROM silver.class_wiki_pages
    FINAL
) AS pg ON (e.page_id = pg.page_id) AND (e.tenant_id = pg.tenant_id)
LEFT JOIN insight.people AS p ON coalesce(lower(pg.author_email), pg.author_id) = p.person_id
WHERE (pg.author_id IS NOT NULL) AND (pg.author_id != '') AND (e.day IS NOT NULL)
;

CREATE OR REPLACE VIEW insight.zoom_person_daily
(
    `person_id` Nullable(String),
    `metric_date` Nullable(Date),
    `user_email` Nullable(String),
    `zoom_calls` UInt64,
    `meeting_hours` Float64
)
AS SELECT
    lower(email) AS person_id,
    date AS metric_date,
    lower(email) AS user_email,
    toUInt64(coalesce(calls_count, 0)) AS zoom_calls,
    toFloat64(coalesce(audio_duration_seconds, 0)) / 3600. AS meeting_hours
FROM silver.class_collab_meeting_activity
WHERE (data_source = 'insight_zoom') AND (email IS NOT NULL) AND (email != '')
;

