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

-- Source measure observations for the unified metrics runtime, task-delivery
-- family. Every measure is emitted through the shape macros in
-- macros/metric_observation_measures.sql.
--
-- The per-issue reconstruction lives in two materialized stages —
-- task_issue_state (pivot + attribution + close) and task_status_intervals
-- (status spans) — because ClickHouse re-inlines every WITH reference: each
-- measure branch below re-evaluates its source CTE, and re-running the
-- field-history pivot per branch multiplied the build's reads by orders of
-- magnitude. With the stages as tables, a branch re-scan costs a small sorted
-- read. This model derives day-grain facts and emits observations; the
-- ordering key mirrors the runtime's filter shape.
--
-- Lifecycle, attribution, and the FINAL-dedup rules are documented on the
-- stage models; worklogs are attributed here via class_task_users (only
-- email-shaped keys pass, tenant rides on the user row).
--
-- Grain per measure:
--   per closed issue (event, for medians): dev_time_hours, resolution_days,
--                    pickup_days
--   day-grain sums (per assignee, close date): tasks_closed, bugs_fixed,
--                    due_date_on_time, due_date_with_due, slip_days_total,
--                    late_count, flow_dev_seconds, flow_lead_seconds
--   day-grain estimation fold inputs: estimation_error_pct (|100 - pct| where
--                    pct = 100 * avg estimate / avg spent over that day's
--                    estimate-carrying closes, kept only for 0 < pct <= 200)
--                    and estimation_samples (1 per qualifying day) — the
--                    registry folds them to 100 - avg error, clamped [0, 100]
--   day-grain sums (per assignee, transition date): close_events,
--                    reopened_within_14d (a close counts as reopened when the
--                    next reopen transition lands within 14 days)
--   day-grain sums (per worklog author / dev-active day): worklog_seconds,
--                    in_progress_seconds (both gated to days with in-progress
--                    time, so the accuracy ratio never counts logging on days
--                    with no tracked development)
--   snapshot (build date): stale_in_progress (open issues idle > 14 days)
--
-- No distinct-count measure, so every row carries subject_key = NULL and the
-- model is a single UNION branch (matching git / ai).

WITH
-- Identity + tenant anchor for worklog attribution. Issue attribution is
-- already resolved on task_issue_state.
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
-- Per closed issue: dev-active seconds (Σ in-progress spans), lead (created →
-- close), pickup (created → first in-progress). Only spans that started
-- before the close count — a reopened issue's live rework belongs to its
-- NEXT close (final_close_at advances when it re-closes), never retroactively
-- to a close already reported. Carries the state a day-grain sum needs, and a
-- precomputed due date so the value expressions stay simple.
-- metric_date = close date.
issue_facts AS (
    SELECT
        s.tenant_id                                                          AS tenant_id,
        s.entity_id                                                          AS entity_id,
        toDate(s.final_close_at)                                             AS metric_date,
        s.issue_id                                                           AS issue_id,
        any(s.issue_type)                                                    AS issue_type,
        -- Count / due-date / estimation measures are defined over issues whose
        -- CURRENT status is done (an issue closed then reopened is not a
        -- present close); duration measures below are defined over every
        -- ever-closed issue. Carry the current category to gate the former.
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
-- Day-grain estimation accuracy inputs: pct compares the day's average
-- original estimate to average time spent over the same set — closed issues
-- (current-status done) that carry both a positive estimate and logged time,
-- so both averages span identical issues. Days whose pct falls outside (0, 200] carry
-- no observation — wildly blown estimates read as unknowable, not as signal.
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
-- Close / reopen transitions from the status spans. A reopen is a transition
-- out of a done category; a close is a transition into one. For each close,
-- the first reopen after it decides reopened-within-14d (spec definition).
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
-- Seconds in an in-progress span per (assignee, calendar day), splitting each
-- span across the days it covers. Denominator for worklog accuracy.
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
-- Worklog is only comparable on days with tracked development, so both sides
-- of the ratio are gated to days with in-progress time (matches the legacy
-- accuracy definition).
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
