{{ config(
    materialized='table',
    engine='MergeTree',
    order_by=['tenant_id', 'assignee_email', 'close_at'],
    partition_by='toYYYYMM(close_date)',
    schema=var('gold_database'),
    tags=['gold'],
    query_settings=metric_serving_query_settings()
) }}

-- Close transitions: one row per time an issue entered a done status, and
-- whether it came back out within a fortnight. An issue closed, reopened and
-- closed again contributes one row per close, so a rate over these rows is a
-- rate over closing acts rather than over issues.

WITH
transitions AS (
    SELECT
        insight_source_id                                                    AS insight_source_id,
        issue_id                                                             AS issue_id,
        interval_start                                                       AS event_at,
        status_category                                                      AS status_category,
        lagInFrame(status_category) OVER (
            PARTITION BY insight_source_id, issue_id ORDER BY interval_start
        )                                                                    AS prev_category
    FROM {{ ref('task_status_spans') }}
),
closes AS (
    SELECT
        insight_source_id                                                    AS insight_source_id,
        issue_id                                                             AS issue_id,
        event_at                                                             AS close_at
    FROM transitions
    WHERE status_category = 'done'
      AND ifNull(prev_category, '') != 'done'
),
reopens AS (
    SELECT
        insight_source_id                                                    AS insight_source_id,
        issue_id                                                             AS issue_id,
        event_at                                                             AS reopen_at
    FROM transitions
    WHERE prev_category = 'done'
      AND ifNull(status_category, '') != 'done'
)

SELECT
    assumeNotNull(any(issues.tenant_id))                                     AS tenant_id,
    closes.insight_source_id                                                 AS insight_source_id,
    closes.issue_id                                                          AS issue_id,
    any(issues.id_readable)                                                  AS id_readable,
    any(issues.title)                                                        AS title,
    assumeNotNull(any(issues.entity_id))                                     AS assignee_email,
    closes.close_at                                                          AS close_at,
    toDate(closes.close_at)                                                  AS close_date,
    -- SAFETY: minIfOrNull, not minIf — over a non-nullable column with nothing
    -- matching, minIf returns the type default and reads as a reopen at epoch.
    toUInt8(ifNull(
        minIfOrNull(reopens.reopen_at, reopens.reopen_at > closes.close_at)
            <= closes.close_at + INTERVAL 14 DAY,
        0))                                                                  AS reopened_within_14d
FROM closes
INNER JOIN {{ ref('task_issue_state') }} AS issues
    ON issues.insight_source_id = closes.insight_source_id
   AND issues.issue_id = closes.issue_id
LEFT JOIN reopens
    ON reopens.insight_source_id = closes.insight_source_id
   AND reopens.issue_id = closes.issue_id
-- SAFETY: the assumeNotNull calls above are sound under this guard.
WHERE issues.tenant_id IS NOT NULL
  AND issues.entity_id IS NOT NULL
  AND issues.entity_id != ''
GROUP BY closes.insight_source_id, closes.issue_id, closes.close_at
