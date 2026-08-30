{{ config(
    materialized='table',
    engine='MergeTree',
    order_by=['tenant_id', 'assignee_email', 'last_status_event_at'],
    partition_by='toYYYYMM(last_status_event_date)',
    schema=var('gold_database'),
    tags=['gold'],
    query_settings=metric_serving_query_settings()
) }}

-- Issues still open: one row per issue that rests outside a done status, dated
-- by the last time its status moved and carrying how long it has sat there.
--
-- INVARIANT: `idle_days` is measured against the moment this table is built,
-- so it answers "how idle is this issue now" and is rewritten on every run —
-- the event time is the status event itself, which never moves.

SELECT
    assumeNotNull(tenant_id)                                                 AS tenant_id,
    insight_source_id                                                        AS insight_source_id,
    issue_id                                                                 AS issue_id,
    id_readable                                                              AS id_readable,
    title                                                                    AS title,
    assumeNotNull(entity_id)                                                 AS assignee_email,
    last_status_event_at                                                     AS last_status_event_at,
    toDate(last_status_event_at)                                             AS last_status_event_date,
    toInt64(dateDiff('day', last_status_event_at, now()))                    AS idle_days
FROM {{ ref('task_issue_state') }}
-- SAFETY: the assumeNotNull calls above are sound under this guard.
WHERE ifNull(status_category, '') != 'done'
  AND tenant_id IS NOT NULL
  AND entity_id IS NOT NULL
  AND entity_id != ''
