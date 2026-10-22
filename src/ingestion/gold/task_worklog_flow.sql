{{ config(
    materialized='table',
    engine=insight_engine('MergeTree'),
    order_by=['tenant_id', 'entity_id', 'account_source_type', 'account_source_id', 'account_id', 'metric_date'],
    schema=var('gold_database'),
    alias='task_worklog_flow',
    tags=['gold'],
    query_settings={
        'join_use_nulls': 1,
        'max_memory_usage': 1610612736,
        'max_threads': 4,
        'max_bytes_before_external_group_by': 805306368,
        'max_bytes_before_external_sort': 805306368
    }
) }}

-- Keyed by the tracker account and its email, the same pair task_issue_state
-- carries, so a day's in-progress time and the worklogs on it meet on one row
-- whichever of the two keys identity resolves.

WITH
-- INVARIANT: no email filter, as in task_issue_state: an author with no email
-- still owns the worklog through the account.
task_users AS (
    SELECT
        tenant_id,
        insight_source_id,
        user_id,
        if(email LIKE '%@%', {{ normalized_email('email') }}, '') AS email
    FROM {{ ref('class_task_users') }} FINAL
),
in_progress_per_day AS (
    SELECT
        s.tenant_id                                                          AS tenant_id,
        s.entity_id                                                          AS entity_id,
        s.account_source_type                                                AS account_source_type,
        s.account_source_id                                                  AS account_source_id,
        s.account_id                                                         AS account_id,
        day                                                                  AS metric_date,
        sum(toFloat64(greatest(toInt64(0),
            dateDiff('second',
                     greatest(i.interval_start, toDateTime(day)),
                     least(i.interval_end, toDateTime(day) + toIntervalDay(1)))))) AS in_progress_seconds
    FROM {{ ref('task_status_spans') }} AS i
    INNER JOIN {{ ref('task_issue_state') }} AS s
        ON s.insight_source_id = i.insight_source_id AND s.issue_id = i.issue_id
    ARRAY JOIN
        arrayMap(d -> toDate(i.interval_start) + toIntervalDay(d),
                 range(toUInt32(greatest(
                     toInt64(0),
                     dateDiff('day', toDate(i.interval_start), toDate(i.interval_end)) + 1
                 )))) AS day
    WHERE i.status_category = 'in_progress'
    GROUP BY s.tenant_id, s.entity_id, s.account_source_type, s.account_source_id, s.account_id, day
),
-- in_progress_per_day inherits the availability filter through its
-- task_issue_state join; worklogs aggregate per person without touching
-- issues, so deletion awareness comes from the class contract's is_deleted
-- (worklog tombstones + deleted parent issue).
worklog_per_day AS (
    SELECT
        u.tenant_id                                                          AS tenant_id,
        u.email                                                              AS entity_id,
        {{ task_account_source_type('w.data_source') }}                      AS account_source_type,
        w.insight_source_id                                                  AS account_source_id,
        w.author_id                                                          AS account_id,
        toDate(w.work_date)                                                  AS metric_date,
        sum(ifNull(w.duration_seconds, 0))                                   AS worklog_seconds
    FROM {{ ref('class_task_worklogs') }} AS w FINAL
    INNER JOIN task_users AS u
        ON u.insight_source_id = w.insight_source_id AND u.user_id = w.author_id
    WHERE w.work_date IS NOT NULL
      AND ifNull(w.is_deleted, 0) = 0
      AND coalesce(w.author_id, '') != ''
    GROUP BY u.tenant_id, entity_id, account_source_type, account_source_id, account_id, metric_date
)
SELECT
    assumeNotNull(coalesce(ip.tenant_id, wl.tenant_id))                      AS tenant_id,
    assumeNotNull(coalesce(ip.entity_id, wl.entity_id))                      AS entity_id,
    assumeNotNull(coalesce(ip.account_source_type, wl.account_source_type))  AS account_source_type,
    assumeNotNull(coalesce(ip.account_source_id, wl.account_source_id))      AS account_source_id,
    assumeNotNull(coalesce(ip.account_id, wl.account_id))                    AS account_id,
    assumeNotNull(coalesce(ip.metric_date, wl.metric_date))                  AS metric_date,
    ifNull(ip.in_progress_seconds, 0)                                        AS in_progress_seconds,
    ifNull(wl.worklog_seconds, 0)                                            AS worklog_seconds
FROM in_progress_per_day AS ip
FULL OUTER JOIN worklog_per_day AS wl
    ON wl.tenant_id = ip.tenant_id
    AND wl.entity_id = ip.entity_id
    AND wl.account_source_type = ip.account_source_type
    AND wl.account_source_id = ip.account_source_id
    AND wl.account_id = ip.account_id
    AND wl.metric_date = ip.metric_date
