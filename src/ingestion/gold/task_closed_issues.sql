{{ config(
    materialized='table',
    engine='MergeTree',
    order_by=['tenant_id', 'assignee_email', 'closed_at'],
    partition_by='toYYYYMM(closed_date)',
    schema=var('gold_database'),
    tags=['gold'],
    query_settings=metric_serving_query_settings(join_use_nulls=1)
) }}

-- Closed issues: one row per issue that reached a close, carrying the outcome
-- an assignee delivered and the durations the work took.
--
-- INVARIANT: an issue reopened after its close keeps its durations and loses
-- its outcome flags — `is_closed` says whether the issue rests in a done
-- status now, and every outcome flag is conjoined with it.
--
-- INVARIANT: the config pins join_use_nulls, so an issue with no in-progress
-- span reads as absent rather than as one starting at the epoch.

WITH
closed AS (
    SELECT
        assumeNotNull(tenant_id)                                             AS tenant_id,
        assumeNotNull(entity_id)                                             AS assignee_email,
        insight_source_id                                                    AS insight_source_id,
        data_source                                                          AS data_source,
        issue_id                                                             AS issue_id,
        id_readable                                                          AS id_readable,
        title                                                                AS title,
        issue_kind                                                           AS issue_kind,
        issue_type_key                                                       AS issue_type_key,
        issue_type_name                                                      AS issue_type_name,
        toUInt8(status_category = 'done')                                    AS is_done,
        due_date                                                             AS due_date,
        time_estimate_seconds                                                AS estimate_seconds,
        time_spent_seconds                                                   AS spent_seconds,
        created_at                                                           AS created_at,
        assumeNotNull(final_close_at)                                        AS closed_at
    FROM {{ ref('task_issue_state') }}
    -- SAFETY: the assumeNotNull calls above are all sound under this guard.
    WHERE final_close_at IS NOT NULL
      AND tenant_id IS NOT NULL
      AND entity_id IS NOT NULL
      AND entity_id != ''
),
-- Development time is the in-progress spans that began before the close; one
-- opened afterwards belongs to whatever happened next, not to this delivery.
in_progress_before_close AS (
    SELECT
        spans.insight_source_id                                              AS insight_source_id,
        spans.issue_id                                                       AS issue_id,
        sum(spans.duration_seconds)                                          AS dev_seconds,
        min(spans.interval_start)                                            AS first_in_progress_at
    FROM {{ ref('task_status_spans') }} AS spans
    INNER JOIN closed
        ON closed.insight_source_id = spans.insight_source_id
       AND closed.issue_id = spans.issue_id
    WHERE spans.status_category = 'in_progress'
      AND spans.interval_start < closed.closed_at
    GROUP BY spans.insight_source_id, spans.issue_id
)

SELECT
    closed.tenant_id                                                         AS tenant_id,
    closed.insight_source_id                                                 AS insight_source_id,
    closed.issue_id                                                          AS issue_id,
    closed.id_readable                                                       AS id_readable,
    closed.title                                                             AS title,
    closed.assignee_email                                                    AS assignee_email,
    closed.closed_at                                                         AS closed_at,
    toDate(closed.closed_at)                                                 AS closed_date,
    ifNull(closed.issue_type_key, '__unknown__')                             AS issue_type,
    ifNull(closed.issue_type_name, 'Type unknown')                           AS issue_type_label,
    closed.data_source                                                       AS source,
    closed.data_source                                                       AS source_label,
    closed.is_done                                                           AS is_closed,
    toUInt8(closed.is_done AND closed.issue_kind = 'bug')                    AS is_bug,
    -- A type the tracker reports but the contract cannot classify is neither a
    -- bug nor a known non-bug; it counts only in the closed total.
    toUInt8(closed.is_done AND closed.issue_kind = 'other')                  AS is_non_bug,
    toUInt8(closed.is_done AND closed.due_date IS NOT NULL)                  AS has_due_date,
    toUInt8(closed.is_done
            AND ifNull(toDate(closed.closed_at) <= closed.due_date, 0))      AS on_time,
    toUInt8(closed.is_done
            AND ifNull(toDate(closed.closed_at) > closed.due_date, 0))       AS is_late,
    if(closed.is_done AND ifNull(toDate(closed.closed_at) > closed.due_date, 0),
       toFloat64(dateDiff('day', closed.due_date, toDate(closed.closed_at))),
       CAST(NULL AS Nullable(Float64)))                                      AS slip_days,
    ifNull(spans.dev_seconds, toFloat64(0))                                  AS dev_seconds,
    ifNull(spans.dev_seconds, toFloat64(0)) / 3600.0                         AS dev_hours,
    toFloat64(greatest(toInt64(0),
        dateDiff('second', closed.created_at, closed.closed_at)))            AS lead_seconds,
    toFloat64(greatest(toInt64(0),
        dateDiff('second', closed.created_at, closed.closed_at))) / 86400.0  AS resolution_days,
    -- An issue that never entered an in-progress status before its close has no
    -- pickup to time, which is not the same as having been picked up at once.
    if(spans.first_in_progress_at IS NULL,
       CAST(NULL AS Nullable(Float64)),
       toFloat64(greatest(toInt64(0),
           dateDiff('second', closed.created_at, spans.first_in_progress_at)))
           / 86400.0)                                                        AS pickup_days,
    closed.estimate_seconds                                                  AS estimate_seconds,
    closed.spent_seconds                                                     AS spent_seconds
FROM closed
LEFT JOIN in_progress_before_close AS spans
    ON spans.insight_source_id = closed.insight_source_id
   AND spans.issue_id = closed.issue_id
