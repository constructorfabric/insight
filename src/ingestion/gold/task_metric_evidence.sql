{{ config(
    materialized='table',
    engine='MergeTree',
    order_by=['tenant_id', 'source_key', 'measure_key', 'entity_id', 'metric_date', 'record_id'],
    schema='insight',
    alias='task_metric_evidence',
    tags=['gold'],
    query_settings={
        'max_memory_usage': 1610612736,
        'max_threads': 4,
        'max_bytes_before_external_group_by': 805306368,
        'max_bytes_before_external_sort': 805306368
    }
) }}


WITH
issue_state AS (
    SELECT *
    FROM {{ ref('task_issue_state') }}
),
status_intervals AS (
    SELECT *
    FROM {{ ref('task_status_spans') }}
),
issue_facts AS (
    SELECT
        s.tenant_id                                                          AS tenant_id,
        s.entity_id                                                          AS entity_id,
        s.insight_source_id                                                   AS insight_source_id,
        toDate(s.final_close_at)                                             AS metric_date,
        any(s.final_close_at)                                                AS observed_at,
        s.issue_id                                                           AS issue_id,
        any(s.issue_type)                                                    AS issue_type,
        any(s.status_category) = 'done'                                      AS is_done,
        toDate(s.final_close_at)                                             AS close_date,
        any(s.due_date)                                                      AS due_date,
        any(s.time_estimate_seconds)                                         AS time_estimate_seconds,
        any(s.time_spent_seconds)                                            AS time_spent_seconds,
        sumIf(i.duration_seconds, i.interval_start < s.final_close_at)      AS dev_seconds,
        if(any(s.created_at) IS NULL,
           CAST(NULL AS Nullable(Float64)),
           toFloat64(greatest(toInt64(0),
               dateDiff('second', any(s.created_at), any(s.final_close_at))))) AS lead_seconds,
        if(any(s.created_at) IS NULL
               OR minIf(i.interval_start, i.interval_start < s.final_close_at) IS NULL,
           CAST(NULL AS Nullable(Float64)),
           toFloat64(greatest(toInt64(0),
               dateDiff('second', any(s.created_at),
                        minIf(i.interval_start, i.interval_start < s.final_close_at))))) AS pickup_seconds,
        CAST([] AS Array(Tuple(key String, value String, label Nullable(String)))) AS no_dimensions
    FROM issue_state AS s
    LEFT JOIN status_intervals AS i
        ON i.insight_source_id = s.insight_source_id
        AND i.issue_id = s.issue_id
        AND i.status_category = 'in_progress'
    WHERE s.final_close_at IS NOT NULL
    GROUP BY s.tenant_id, s.entity_id, s.insight_source_id, s.issue_id, toDate(s.final_close_at)
),
issue_item_evidence AS (
    SELECT
        tenant_id,
        entity_id,
        insight_source_id,
        toDate(final_close_at) AS metric_date,
        final_close_at AS observed_at,
        issue_id,
        issue_type,
        item_measure.1 AS measure_key,
        toFloat64(item_measure.2) AS contribution,
        CAST([] AS Array(Tuple(key String, value String, label Nullable(String)))) AS no_dimensions
    FROM issue_state
    ARRAY JOIN arrayConcat(
        [tuple('tasks_closed', toFloat64(1))],
        if(issue_type = 'Bug', [tuple('bugs_fixed', toFloat64(1))], []),
        if(
            due_date IS NOT NULL AND toDate(final_close_at) <= due_date,
            [tuple('due_date_on_time', toFloat64(1))],
            []
        ),
        if(due_date IS NOT NULL, [tuple('due_date_with_due', toFloat64(1))], []),
        if(
            due_date IS NOT NULL AND toDate(final_close_at) > due_date,
            [
                tuple(
                    'slip_days_total',
                    toFloat64(dateDiff('day', due_date, toDate(final_close_at)))
                ),
                tuple('late_count', toFloat64(1))
            ],
            []
        )
    ) AS item_measure
    WHERE final_close_at IS NOT NULL
      AND status_category = 'done'
),
estimation_day AS (
    SELECT
        tenant_id,
        entity_id,
        metric_date,
        100 * avgIf(time_estimate_seconds, is_done AND ifNull(time_estimate_seconds, 0) > 0 AND time_spent_seconds IS NOT NULL)
            / nullIf(avgIf(time_spent_seconds, is_done AND ifNull(time_estimate_seconds, 0) > 0 AND time_spent_seconds IS NOT NULL), 0)
            AS estimation_pct,
        CAST([] AS Array(Tuple(key String, value String, label Nullable(String)))) AS no_dimensions
    FROM issue_facts
    GROUP BY tenant_id, entity_id, metric_date
),
transitions AS (
    SELECT
        insight_source_id,
        issue_id,
        interval_start AS event_at,
        status_category,
        lagInFrame(status_category) OVER (
            PARTITION BY insight_source_id, issue_id ORDER BY interval_start
        ) AS prev_category
    FROM status_intervals
),
closes AS (
    SELECT insight_source_id, issue_id, event_at AS close_at
    FROM transitions
    WHERE status_category = 'done' AND (prev_category IS NULL OR prev_category != 'done')
),
reopens AS (
    SELECT insight_source_id, issue_id, event_at AS reopen_at
    FROM transitions
    WHERE prev_category = 'done' AND (status_category != 'done' OR status_category IS NULL)
),
close_reopen AS (
    SELECT
        s.tenant_id                                                          AS tenant_id,
        s.entity_id                                                          AS entity_id,
        toDate(c.close_at)                                                   AS metric_date,
        toFloat64(1)                                                         AS close_event,
        if(minIf(r.reopen_at, r.reopen_at > c.close_at) IS NOT NULL
           AND minIf(r.reopen_at, r.reopen_at > c.close_at) <= c.close_at + INTERVAL 14 DAY,
           toFloat64(1), CAST(NULL AS Nullable(Float64)))                    AS reopened_14d,
        CAST([] AS Array(Tuple(key String, value String, label Nullable(String)))) AS no_dimensions
    FROM closes AS c
    INNER JOIN issue_state AS s
        ON s.insight_source_id = c.insight_source_id AND s.issue_id = c.issue_id
    LEFT JOIN reopens AS r
        ON r.insight_source_id = c.insight_source_id AND r.issue_id = c.issue_id
    GROUP BY s.tenant_id, s.entity_id, c.insight_source_id, c.issue_id, c.close_at
),
worklog_flow AS (
    SELECT
        tenant_id,
        entity_id,
        metric_date,
        in_progress_seconds,
        worklog_seconds,
        CAST([] AS Array(Tuple(key String, value String, label Nullable(String)))) AS no_dimensions
    FROM {{ ref('task_worklog_flow') }}
),
stale AS (
    SELECT
        s.tenant_id                                                          AS tenant_id,
        s.entity_id                                                          AS entity_id,
        toDate(s.last_status_event_at)                                       AS metric_date,
        toFloat64(count())                                                   AS stale_count,
        CAST([] AS Array(Tuple(key String, value String, label Nullable(String)))) AS no_dimensions
    FROM issue_state AS s
    WHERE (s.status_category IS NULL OR s.status_category != 'done')
      AND s.last_status_event_at IS NOT NULL
      AND dateDiff('day', s.last_status_event_at, now()) > 14
    GROUP BY s.tenant_id, s.entity_id, toDate(s.last_status_event_at)
),
value_measures AS (
    {{ sum_measure('estimation_error_pct', 'estimation_day', 'if(estimation_pct > 0 AND estimation_pct <= 200, abs(100 - estimation_pct), NULL)', 'no_dimensions') }}

    UNION ALL

    {{ sum_measure('estimation_samples', 'estimation_day', 'if(estimation_pct > 0 AND estimation_pct <= 200, 1, NULL)', 'no_dimensions') }}

    UNION ALL

    {{ sum_measure('flow_dev_seconds', 'issue_facts', 'if(ifNull(dev_seconds, 0) > 0 AND ifNull(lead_seconds, 0) > 0, dev_seconds, NULL)', 'no_dimensions') }}

    UNION ALL

    {{ sum_measure('flow_lead_seconds', 'issue_facts', 'if(ifNull(dev_seconds, 0) > 0 AND ifNull(lead_seconds, 0) > 0, lead_seconds, NULL)', 'no_dimensions') }}

    UNION ALL

    {{ sum_measure('close_events', 'close_reopen', 'close_event', 'no_dimensions') }}

    UNION ALL

    {{ sum_measure('reopened_within_14d', 'close_reopen', 'reopened_14d', 'no_dimensions') }}

    UNION ALL

    {{ sum_measure('worklog_seconds', 'worklog_flow', 'worklog_seconds', 'no_dimensions', where='in_progress_seconds > 0') }}

    UNION ALL

    {{ sum_measure('in_progress_seconds', 'worklog_flow', 'in_progress_seconds', 'no_dimensions', where='in_progress_seconds > 0') }}

    UNION ALL

    {{ sum_measure('stale_in_progress', 'stale', 'stale_count', 'no_dimensions') }}
)
SELECT
    assumeNotNull(tenant_id) AS tenant_id,
    'task' AS source_key,
    'person' AS entity_type,
    assumeNotNull(entity_id) AS entity_id,
    assumeNotNull(metric_date) AS metric_date,
    CAST(NULL AS Nullable(DateTime64(3))) AS observed_at,
    measure_key,
    concat(
        toString(metric_date),
        ':',
        measure_key,
        ':',
        hex(sipHash128(toString(arrayMap(d -> tuple(d.1, d.2), dimensions))))
    ) AS record_id,
    measure_key AS record_kind,
    'derived_population' AS granularity,
    replaceAll(measure_key, '_', ' ') AS record_label,
    value AS contribution,
    CAST(NULL AS Nullable(String)) AS subject_key,
    dimensions,
    CAST(map() AS Map(String, String)) AS details
FROM value_measures
WHERE tenant_id IS NOT NULL
  AND entity_id IS NOT NULL
  AND metric_date IS NOT NULL

UNION ALL

SELECT
    assumeNotNull(tenant_id) AS tenant_id,
    'task' AS source_key,
    'person' AS entity_type,
    assumeNotNull(entity_id) AS entity_id,
    assumeNotNull(metric_date) AS metric_date,
    toNullable(toDateTime64(observed_at, 3)) AS observed_at,
    measure_key,
    concat(toString(insight_source_id), ':', toString(issue_id), ':', measure_key) AS record_id,
    'issue' AS record_kind,
    'event' AS granularity,
    toString(issue_id) AS record_label,
    toNullable(contribution) AS contribution,
    CAST(NULL AS Nullable(String)) AS subject_key,
    no_dimensions AS dimensions,
    map(
        'ref', toString(issue_id),
        'issue_type', ifNull(issue_type, '')
    ) AS details
FROM issue_item_evidence
WHERE tenant_id IS NOT NULL
  AND entity_id IS NOT NULL
  AND entity_id != ''
  AND metric_date IS NOT NULL

UNION ALL

SELECT
    assumeNotNull(tenant_id) AS tenant_id,
    'task' AS source_key,
    'person' AS entity_type,
    assumeNotNull(entity_id) AS entity_id,
    assumeNotNull(metric_date) AS metric_date,
    toNullable(toDateTime64(observed_at, 3)) AS observed_at,
    duration_measure.1 AS measure_key,
    concat(toString(insight_source_id), ':', toString(issue_id), ':', duration_measure.1) AS record_id,
    'issue' AS record_kind,
    'event' AS granularity,
    toString(issue_id) AS record_label,
    toNullable(toFloat64(duration_measure.2)) AS contribution,
    CAST(NULL AS Nullable(String)) AS subject_key,
    no_dimensions AS dimensions,
    map(
        'ref', toString(issue_id),
        'issue_type', ifNull(issue_type, '')
    ) AS details
FROM issue_facts
ARRAY JOIN arrayConcat(
    if(ifNull(dev_seconds, 0) > 0, [tuple('dev_time_hours', toFloat64(dev_seconds / 3600.0))], []),
    if(ifNull(lead_seconds, 0) > 0, [tuple('resolution_days', toFloat64(lead_seconds / 86400.0))], []),
    if(pickup_seconds IS NOT NULL, [tuple('pickup_days', toFloat64(pickup_seconds / 86400.0))], [])
) AS duration_measure
WHERE tenant_id IS NOT NULL
  AND entity_id IS NOT NULL
  AND entity_id != ''
  AND metric_date IS NOT NULL
