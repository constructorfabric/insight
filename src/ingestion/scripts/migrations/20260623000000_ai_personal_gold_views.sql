DROP VIEW IF EXISTS insight.ai_dev_tool_daily;
CREATE VIEW insight.ai_dev_tool_daily AS
SELECT
    lower(c.email) AS person_id,
    p.org_unit_id AS org_unit_id,
    c.day AS metric_date,
    c.tool AS tool,
    c.source AS source,
    c.source_id AS source_id,
    sum(toFloat64(coalesce(c.lines_added, 0))) AS accepted_lines_added,
    sum(toFloat64(coalesce(c.lines_removed, 0))) AS accepted_lines_removed,
    if(countIf(c.tool_use_accepted IS NOT NULL) > 0, sumIf(toFloat64(c.tool_use_accepted), c.tool_use_accepted IS NOT NULL), CAST(NULL AS Nullable(Float64))) AS tool_use_accepted,
    if(countIf(c.tool_use_offered IS NOT NULL) > 0, sumIf(toFloat64(c.tool_use_offered), c.tool_use_offered IS NOT NULL), CAST(NULL AS Nullable(Float64))) AS tool_use_offered,
    if(countIf(c.cost_cents IS NOT NULL) > 0, sumIf(toFloat64(c.cost_cents), c.cost_cents IS NOT NULL), CAST(NULL AS Nullable(Float64))) AS cost_cents,
    if(c.tool IN ('claude_code', 'codex') AND countIf(c.session_count IS NOT NULL) > 0, sumIf(toFloat64(c.session_count), c.session_count IS NOT NULL), CAST(NULL AS Nullable(Float64))) AS dev_agent_conversations,
    toFloat64(1) AS active_day
FROM silver.class_ai_dev_usage AS c
LEFT JOIN insight.people AS p ON lower(c.email) = p.person_id
WHERE c.email IS NOT NULL
  AND c.email != ''
GROUP BY person_id, org_unit_id, metric_date, tool, source, source_id;

DROP VIEW IF EXISTS insight.ai_dev_tool_person_period;
CREATE VIEW insight.ai_dev_tool_person_period AS
SELECT
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
FROM (
    SELECT
        person_id,
        tool,
        max(metric_date) AS last_metric_date,
        sum(accepted_lines_added) AS accepted_lines_added,
        sum(accepted_lines_removed) AS accepted_lines_removed,
        if(countIf(tool_use_accepted IS NOT NULL) > 0, sumIf(tool_use_accepted, tool_use_accepted IS NOT NULL), CAST(NULL AS Nullable(Float64))) AS tool_use_accepted,
        if(countIf(tool_use_offered IS NOT NULL) > 0, sumIf(tool_use_offered, tool_use_offered IS NOT NULL), CAST(NULL AS Nullable(Float64))) AS tool_use_offered,
        if(countIf(cost_cents IS NOT NULL) > 0, sumIf(cost_cents, cost_cents IS NOT NULL), CAST(NULL AS Nullable(Float64))) AS cost_cents,
        if(countIf(dev_agent_conversations IS NOT NULL) > 0, sumIf(dev_agent_conversations, dev_agent_conversations IS NOT NULL), CAST(NULL AS Nullable(Float64))) AS dev_agent_conversations,
        uniqExact(metric_date) AS active_days
    FROM insight.ai_dev_tool_daily
    GROUP BY person_id, tool
) d
LEFT JOIN insight.people AS p ON d.person_id = p.person_id;

DROP VIEW IF EXISTS insight.ai_assistant_tool_daily;
CREATE VIEW insight.ai_assistant_tool_daily AS
SELECT
    lower(a.email) AS person_id,
    p.org_unit_id AS org_unit_id,
    a.day AS metric_date,
    a.tool AS tool,
    a.surface AS surface,
    a.source AS source,
    a.source_id AS source_id,
    if(countIf(a.message_count IS NOT NULL) > 0, sumIf(toFloat64(a.message_count), a.message_count IS NOT NULL), CAST(NULL AS Nullable(Float64))) AS assistant_messages,
    if(countIf(a.action_count IS NOT NULL) > 0, sumIf(toFloat64(a.action_count), a.action_count IS NOT NULL), CAST(NULL AS Nullable(Float64))) AS assistant_actions,
    if(a.surface = 'chat' AND countIf(a.conversation_count IS NOT NULL) > 0, sumIf(toFloat64(a.conversation_count), a.conversation_count IS NOT NULL), CAST(NULL AS Nullable(Float64))) AS chat_assistant_conversations
FROM silver.class_ai_assistant_usage AS a
LEFT JOIN insight.people AS p ON lower(a.email) = p.person_id
WHERE a.email IS NOT NULL
  AND a.email != ''
GROUP BY person_id, org_unit_id, metric_date, tool, surface, source, source_id;

DROP VIEW IF EXISTS insight.ai_person_counter_daily;
CREATE VIEW insight.ai_person_counter_daily AS
SELECT
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
FROM (
    SELECT
        person_id,
        metric_date,
        if(countIf(ai_accepted_lines IS NOT NULL) > 0, sumIf(ai_accepted_lines, ai_accepted_lines IS NOT NULL), CAST(NULL AS Nullable(Float64))) AS ai_accepted_lines,
        if(countIf(ai_removed_lines IS NOT NULL) > 0, sumIf(ai_removed_lines, ai_removed_lines IS NOT NULL), CAST(NULL AS Nullable(Float64))) AS ai_removed_lines,
        if(countIf(ai_active_days IS NOT NULL) > 0, maxIf(ai_active_days, ai_active_days IS NOT NULL), CAST(NULL AS Nullable(Float64))) AS ai_active_days,
        if(countIf(ai_cost_cents IS NOT NULL) > 0, sumIf(ai_cost_cents, ai_cost_cents IS NOT NULL), CAST(NULL AS Nullable(Float64))) AS ai_cost_cents,
        if(countIf(ai_accepted_edit_actions IS NOT NULL) > 0, sumIf(ai_accepted_edit_actions, ai_accepted_edit_actions IS NOT NULL), CAST(NULL AS Nullable(Float64))) AS ai_accepted_edit_actions,
        if(countIf(ai_tool_acceptance_offered IS NOT NULL) > 0, sumIf(ai_tool_acceptance_offered, ai_tool_acceptance_offered IS NOT NULL), CAST(NULL AS Nullable(Float64))) AS ai_tool_acceptance_offered,
        if(countIf(ai_tool_acceptance_accepted IS NOT NULL) > 0, sumIf(ai_tool_acceptance_accepted, ai_tool_acceptance_accepted IS NOT NULL), CAST(NULL AS Nullable(Float64))) AS ai_tool_acceptance_accepted,
        if(countIf(ai_assistant_messages IS NOT NULL) > 0, sumIf(ai_assistant_messages, ai_assistant_messages IS NOT NULL), CAST(NULL AS Nullable(Float64))) AS ai_assistant_messages,
        if(countIf(ai_assistant_actions IS NOT NULL) > 0, sumIf(ai_assistant_actions, ai_assistant_actions IS NOT NULL), CAST(NULL AS Nullable(Float64))) AS ai_assistant_actions,
        if(countIf(ai_dev_agent_conversations IS NOT NULL) > 0, sumIf(ai_dev_agent_conversations, ai_dev_agent_conversations IS NOT NULL), CAST(NULL AS Nullable(Float64))) AS ai_dev_agent_conversations,
        if(countIf(ai_chat_assistant_conversations IS NOT NULL) > 0, sumIf(ai_chat_assistant_conversations, ai_chat_assistant_conversations IS NOT NULL), CAST(NULL AS Nullable(Float64))) AS ai_chat_assistant_conversations
    FROM (
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
            CAST(NULL AS Nullable(Float64)) AS ai_assistant_messages,
            CAST(NULL AS Nullable(Float64)) AS ai_assistant_actions,
            dev_agent_conversations AS ai_dev_agent_conversations,
            CAST(NULL AS Nullable(Float64)) AS ai_chat_assistant_conversations
        FROM insight.ai_dev_tool_daily
        UNION ALL
        SELECT
            person_id,
            metric_date,
            CAST(NULL AS Nullable(Float64)) AS ai_accepted_lines,
            CAST(NULL AS Nullable(Float64)) AS ai_removed_lines,
            CAST(NULL AS Nullable(Float64)) AS ai_active_days,
            CAST(NULL AS Nullable(Float64)) AS ai_cost_cents,
            CAST(NULL AS Nullable(Float64)) AS ai_accepted_edit_actions,
            CAST(NULL AS Nullable(Float64)) AS ai_tool_acceptance_offered,
            CAST(NULL AS Nullable(Float64)) AS ai_tool_acceptance_accepted,
            assistant_messages AS ai_assistant_messages,
            assistant_actions AS ai_assistant_actions,
            CAST(NULL AS Nullable(Float64)) AS ai_dev_agent_conversations,
            chat_assistant_conversations AS ai_chat_assistant_conversations
        FROM insight.ai_assistant_tool_daily
    ) raw
    GROUP BY person_id, metric_date
) d
LEFT JOIN insight.people AS p ON d.person_id = p.person_id;

DROP VIEW IF EXISTS insight.ai_cost_person_period;
CREATE VIEW insight.ai_cost_person_period AS
SELECT
    d.person_id AS person_id,
    p.org_unit_id AS org_unit_id,
    d.tool AS tool,
    d.last_metric_date AS last_metric_date,
    d.total_cost_cents AS total_cost_cents
FROM (
    SELECT
        person_id,
        tool,
        max(metric_date) AS last_metric_date,
        sum(cost_cents) AS total_cost_cents
    FROM insight.ai_dev_tool_daily
    WHERE cost_cents IS NOT NULL
    GROUP BY person_id, tool
) d
LEFT JOIN insight.people AS p ON d.person_id = p.person_id;

DROP VIEW IF EXISTS insight.ai_dev_tool_trend;
CREATE VIEW insight.ai_dev_tool_trend AS
SELECT
    person_id,
    org_unit_id,
    metric_date,
    tool,
    sum(accepted_lines_added) AS accepted_lines_added
FROM insight.ai_dev_tool_daily
GROUP BY person_id, org_unit_id, metric_date, tool;
