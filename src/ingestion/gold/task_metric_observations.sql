{{ config(
    materialized='table',
    engine='MergeTree',
    order_by=['source_key', 'measure_key', 'entity_id', 'metric_date'],
    schema='insight',
    alias='task_metric_observations',
    tags=['gold'],
    query_settings={
        'max_memory_usage': 1610612736,
        'max_threads': 4,
        'max_bytes_before_external_group_by': 805306368,
        'max_bytes_before_external_sort': 805306368
    }
) }}

-- Task-delivery measure observations for the unified metrics runtime; every
-- measure is emitted through macros/metric_observation_measures.sql.
--
-- The per-issue reconstruction is materialized upstream (task_issue_state,
-- task_status_intervals): ClickHouse re-inlines every WITH reference, so the
-- measure branches would otherwise re-run it once per branch.
--
-- Grain: event rows per closed issue for the median metrics (dev_time_hours,
-- resolution_days, pickup_days); day-grain sums for everything else;
-- stale_in_progress is a build-date snapshot. No distinct-count measure, so
-- subject_key is always NULL.

WITH
-- Worklog attribution (account id → lowercased email, tenant on the same
-- row); issue attribution is already resolved on task_issue_state.
task_users AS (
    SELECT
        tenant_id,
        insight_source_id,
        user_id,
        lower(email) AS email
    FROM {{ ref('class_task_users') }} FINAL
    WHERE email LIKE '%@%'
),
issue_state AS (
    SELECT *
    FROM {{ ref('task_issue_state') }}
),
status_intervals AS (
    SELECT *
    FROM {{ ref('task_status_intervals') }}
),
-- Per closed issue (metric_date = close date): dev seconds, lead, pickup.
-- Only spans started before the close count — live rework on a reopened
-- issue belongs to its next close, never retroactively to one reported.
issue_facts AS (
    SELECT
        s.tenant_id                                                          AS tenant_id,
        s.entity_id                                                          AS entity_id,
        toDate(s.final_close_at)                                             AS metric_date,
        s.issue_id                                                           AS issue_id,
        any(s.issue_type)                                                    AS issue_type,
        -- Gates count/due-date/estimation measures to CURRENT-status-done
        -- issues; duration measures cover every ever-closed issue.
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
    GROUP BY s.tenant_id, s.entity_id, s.issue_id, toDate(s.final_close_at)
),
-- Day-grain estimation pct: avg estimate / avg spent, both averages pinned to
-- the SAME set (done, positive estimate, logged time). pct outside (0, 200]
-- emits nothing — blown estimates read as unknowable, not as signal.
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
-- A close is a transition into a done category, a reopen out of one; the
-- first reopen after a close decides reopened-within-14d.
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
    GROUP BY s.tenant_id, s.entity_id, c.issue_id, c.close_at
),
-- In-progress seconds per (assignee, calendar day); worklog-accuracy
-- denominator.
in_progress_per_day AS (
    SELECT
        s.tenant_id                                                          AS tenant_id,
        s.entity_id                                                          AS entity_id,
        day                                                                  AS metric_date,
        sum(toFloat64(greatest(toInt64(0),
            dateDiff('second',
                     greatest(i.interval_start, toDateTime(day)),
                     least(i.interval_end, toDateTime(day) + toIntervalDay(1)))))) AS in_progress_seconds
    FROM status_intervals AS i
    INNER JOIN issue_state AS s
        ON s.insight_source_id = i.insight_source_id AND s.issue_id = i.issue_id
    ARRAY JOIN
        arrayMap(d -> toDate(i.interval_start) + toIntervalDay(d),
                 range(toUInt32(dateDiff('day', toDate(i.interval_start), toDate(i.interval_end)) + 1))) AS day
    WHERE i.status_category = 'in_progress'
    GROUP BY s.tenant_id, s.entity_id, day
),
worklog_per_day AS (
    SELECT
        u.tenant_id                                                          AS tenant_id,
        u.email                                                              AS entity_id,
        toDate(w.work_date)                                                  AS metric_date,
        sum(ifNull(w.duration_seconds, 0))                                   AS worklog_seconds
    FROM {{ ref('class_task_worklogs') }} AS w FINAL
    INNER JOIN task_users AS u
        ON u.insight_source_id = w.insight_source_id AND u.user_id = w.author_id
    WHERE w.work_date IS NOT NULL
    GROUP BY u.tenant_id, u.email, toDate(w.work_date)
),
-- Both sides of the accuracy ratio are gated to days with in-progress time —
-- logging on a day with no tracked development is not comparable.
worklog_flow AS (
    SELECT
        coalesce(ip.tenant_id, wl.tenant_id)                                 AS tenant_id,
        coalesce(ip.entity_id, wl.entity_id)                                 AS entity_id,
        coalesce(ip.metric_date, wl.metric_date)                             AS metric_date,
        ifNull(ip.in_progress_seconds, 0)                                    AS in_progress_seconds,
        ifNull(wl.worklog_seconds, 0)                                        AS worklog_seconds,
        CAST([] AS Array(Tuple(key String, value String, label Nullable(String)))) AS no_dimensions
    FROM in_progress_per_day AS ip
    FULL OUTER JOIN worklog_per_day AS wl
        ON wl.tenant_id = ip.tenant_id
        AND wl.entity_id = ip.entity_id
        AND wl.metric_date = ip.metric_date
),
-- Snapshot at build date: open (non-done) issues idle more than 14 days.
stale AS (
    SELECT
        s.tenant_id                                                          AS tenant_id,
        s.entity_id                                                          AS entity_id,
        today()                                                              AS metric_date,
        toFloat64(count())                                                   AS stale_count,
        CAST([] AS Array(Tuple(key String, value String, label Nullable(String)))) AS no_dimensions
    FROM issue_state AS s
    WHERE (s.status_category IS NULL OR s.status_category != 'done')
      AND s.last_status_event_at IS NOT NULL
      AND dateDiff('day', s.last_status_event_at, now()) > 14
    GROUP BY s.tenant_id, s.entity_id
),
value_measures AS (
    {{ sum_measure('tasks_closed', 'issue_facts', 'if(is_done, 1, NULL)', 'no_dimensions') }}

    UNION ALL

    {{ sum_measure('bugs_fixed', 'issue_facts', "if(is_done AND issue_type = 'Bug', 1, NULL)", 'no_dimensions') }}

    UNION ALL

    {{ sum_measure('due_date_on_time', 'issue_facts', 'if(is_done AND due_date IS NOT NULL AND close_date <= due_date, 1, NULL)', 'no_dimensions') }}

    UNION ALL

    {{ sum_measure('due_date_with_due', 'issue_facts', 'if(is_done AND due_date IS NOT NULL, 1, NULL)', 'no_dimensions') }}

    UNION ALL

    {{ sum_measure('slip_days_total', 'issue_facts', 'if(is_done AND due_date IS NOT NULL AND close_date > due_date, toFloat64(dateDiff(\'day\', due_date, close_date)), NULL)', 'no_dimensions') }}

    UNION ALL

    {{ sum_measure('late_count', 'issue_facts', 'if(is_done AND due_date IS NOT NULL AND close_date > due_date, 1, NULL)', 'no_dimensions') }}

    UNION ALL

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

    UNION ALL

    {{ event_measure('dev_time_hours', 'issue_facts', 'dev_seconds / 3600.0', 'no_dimensions', where='ifNull(dev_seconds, 0) > 0') }}

    UNION ALL

    {{ event_measure('resolution_days', 'issue_facts', 'lead_seconds / 86400.0', 'no_dimensions', where='ifNull(lead_seconds, 0) > 0') }}

    UNION ALL

    {{ event_measure('pickup_days', 'issue_facts', 'pickup_seconds / 86400.0', 'no_dimensions', where='pickup_seconds IS NOT NULL') }}
)
SELECT
    assumeNotNull(tenant_id) AS tenant_id,
    'task' AS source_key,
    'person' AS entity_type,
    assumeNotNull(entity_id) AS entity_id,
    assumeNotNull(metric_date) AS metric_date,
    CAST(NULL AS Nullable(DateTime64(3))) AS observed_at,
    measure_key,
    value,
    CAST(NULL AS Nullable(String)) AS subject_key,
    dimensions
FROM value_measures
WHERE tenant_id IS NOT NULL
  AND entity_id IS NOT NULL
  AND metric_date IS NOT NULL
