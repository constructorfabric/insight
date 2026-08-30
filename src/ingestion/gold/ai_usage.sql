{{ config(
    materialized='table',
    engine='MergeTree',
    order_by=['tenant_id', 'email', 'usage_date'],
    partition_by='toYYYYMM(usage_date)',
    schema=var('gold_database'),
    settings={'allow_nullable_key': 1},
    tags=['gold'],
    query_settings=metric_serving_query_settings()
) }}

-- AI usage per person and day: what coding assistants and chat assistants each
-- reported, in one relation told apart by `surface`.
--
-- INVARIANT: cost and active days span both kinds of usage and a measure reads
-- one dataset, so the two class relations meet here and not at query time.
--
-- INVARIANT: a counter the source did not report stays NULL rather than
-- becoming zero, so a sum over a person who was never measured is no value.

WITH
dev_usage AS (
    SELECT
        insight_tenant_id AS tenant_id,
        {{ normalized_email('email') }} AS email,
        day AS usage_date,
        tool AS tool,
        'dev' AS surface,
        -- Seat state as of the last read, not as of the usage day: the sources
        -- restate it for every day they re-read. 'unknown' where the source has
        -- no seat lifecycle concept.
        coalesce(seat_status, 'unknown') AS seat_status,
        toNullable(lines_added) AS lines_added,
        lines_removed AS lines_removed,
        tool_use_offered AS tool_use_offered,
        tool_use_accepted AS tool_use_accepted,
        conversation_count AS dev_conversations,
        CAST(NULL AS Nullable(UInt32)) AS assistant_messages,
        CAST(NULL AS Nullable(UInt32)) AS assistant_actions,
        CAST(NULL AS Nullable(UInt32)) AS chat_conversations,
        prs_with_cc_count AS prs_with_assistant,
        prs_total_count AS prs_total,
        cost_cents AS cost_cents
    FROM {{ ref('class_ai_dev_usage') }} FINAL
    WHERE insight_tenant_id IS NOT NULL
      AND email IS NOT NULL
      AND email != ''
      AND day IS NOT NULL
),
assistant_usage AS (
    SELECT
        insight_tenant_id AS tenant_id,
        {{ normalized_email('email') }} AS email,
        day AS usage_date,
        tool AS tool,
        surface AS surface,
        -- Assistant sources report no seat lifecycle, so the same 'unknown' the
        -- dev branch falls back to.
        'unknown' AS seat_status,
        CAST(NULL AS Nullable(UInt32)) AS lines_added,
        CAST(NULL AS Nullable(UInt32)) AS lines_removed,
        CAST(NULL AS Nullable(UInt32)) AS tool_use_offered,
        CAST(NULL AS Nullable(UInt32)) AS tool_use_accepted,
        CAST(NULL AS Nullable(UInt32)) AS dev_conversations,
        message_count AS assistant_messages,
        action_count AS assistant_actions,
        conversation_count AS chat_conversations,
        CAST(NULL AS Nullable(UInt32)) AS prs_with_assistant,
        CAST(NULL AS Nullable(UInt32)) AS prs_total,
        cost_cents AS cost_cents
    FROM {{ ref('class_ai_assistant_usage') }} FINAL
    WHERE insight_tenant_id IS NOT NULL
      AND email IS NOT NULL
      AND email != ''
      AND day IS NOT NULL
),
reported_usage AS (
    SELECT * FROM dev_usage

    UNION ALL

    SELECT * FROM assistant_usage
)

SELECT
    -- SAFETY: both branches admit only non-null keys.
    assumeNotNull(tenant_id) AS tenant_id,
    assumeNotNull(email) AS email,
    assumeNotNull(usage_date) AS usage_date,
    tool AS tool,
    {{ ai_tool_label('tool') }} AS tool_label,
    surface AS surface,
    {{ ai_surface_label('surface') }} AS surface_label,
    seat_status AS seat_status,
    sum(lines_added) AS lines_added,
    sum(lines_removed) AS lines_removed,
    sum(tool_use_offered) AS tool_use_offered,
    sum(tool_use_accepted) AS tool_use_accepted,
    sum(dev_conversations) AS dev_conversations,
    sum(assistant_messages) AS assistant_messages,
    sum(assistant_actions) AS assistant_actions,
    sum(chat_conversations) AS chat_conversations,
    sum(prs_with_assistant) AS prs_with_assistant,
    sum(prs_total) AS prs_total,
    sum(cost_cents) / 100 AS cost_usd
FROM reported_usage
GROUP BY
    tenant_id,
    email,
    usage_date,
    tool,
    tool_label,
    surface,
    surface_label,
    seat_status
