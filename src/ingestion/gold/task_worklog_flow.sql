{{ config(
    materialized='table',
    engine=insight_engine('MergeTree'),
    order_by=['tenant_id', 'entity_id', 'account_source_type', 'account_source_id', 'account_id', 'metric_date', 'insight_source_id', 'issue_id'],
    schema=var('gold_database'),
    alias='task_worklog_flow',
    tags=['gold'],
    query_settings={
        'join_use_nulls': 1,
        'max_memory_usage': 3221225472,
        'max_threads': 4,
        'max_bytes_before_external_group_by': 805306368,
        'max_bytes_before_external_sort': 805306368
    }
) }}

-- One row per person-day (see `task_person_day_group`) and issue: the evidence
-- gates worklog seconds on the in-progress seconds of the whole person-day, so
-- every row carries that day's total beside its own issue's seconds.

WITH
-- Same reads as task_issue_state: the tenant comes from the source, and the
-- author joins LEFT with no email filter, so an author with no email or no
-- directory row still owns the worklog through the account.
source_tenants AS (
    SELECT
        insight_source_id,
        min(tenant_id) AS tenant_id
    FROM {{ ref('class_task_users') }} FINAL
    GROUP BY insight_source_id
),
task_users AS (
    SELECT
        insight_source_id,
        user_id,
        if(email LIKE '%@%', {{ normalized_email('email') }}, '') AS email
    FROM {{ ref('class_task_users') }} FINAL
),
in_progress_per_account_issue_day AS (
    SELECT
        s.tenant_id                                                          AS tenant_id,
        s.entity_id                                                          AS entity_id,
        s.account_source_type                                                AS account_source_type,
        s.account_source_id                                                  AS account_source_id,
        s.account_id                                                         AS account_id,
        i.insight_source_id                                                  AS insight_source_id,
        i.issue_id                                                           AS issue_id,
        any(s.data_source)                                                   AS data_source,
        any(s.id_readable)                                                   AS id_readable,
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
    GROUP BY s.tenant_id, s.entity_id, s.account_source_type, s.account_source_id, s.account_id,
             i.insight_source_id, i.issue_id, day
),
-- in_progress_per_account_issue_day inherits the availability filter through
-- its task_issue_state join; worklogs aggregate per person without touching
-- issues, so deletion awareness comes from the class contract's is_deleted
-- (worklog tombstones + deleted parent issue).
worklog_per_account_issue_day AS (
    SELECT
        t.tenant_id                                                          AS tenant_id,
        coalesce(u.email, '')                                                AS entity_id,
        {{ task_account_source_type('w.data_source') }}                      AS account_source_type,
        w.insight_source_id                                                  AS account_source_id,
        w.author_id                                                          AS account_id,
        w.insight_source_id                                                  AS insight_source_id,
        coalesce(w.issue_id, '')                                             AS issue_id,
        any(w.data_source)                                                   AS data_source,
        any(w.id_readable)                                                   AS id_readable,
        toDate(w.work_date)                                                  AS metric_date,
        sum(ifNull(w.duration_seconds, 0))                                   AS worklog_seconds
    FROM {{ ref('class_task_worklogs') }} AS w FINAL
    INNER JOIN source_tenants AS t
        ON t.insight_source_id = w.insight_source_id
    LEFT JOIN task_users AS u
        ON u.insight_source_id = w.insight_source_id AND u.user_id = w.author_id
    WHERE w.work_date IS NOT NULL
      AND ifNull(w.is_deleted, 0) = 0
      AND coalesce(w.author_id, '') != ''
    GROUP BY t.tenant_id, entity_id, account_source_type, account_source_id, account_id,
             insight_source_id, issue_id, metric_date
),
account_issue_days AS (
    SELECT
        tenant_id,
        entity_id,
        account_source_type,
        account_source_id,
        account_id,
        insight_source_id,
        issue_id,
        data_source,
        id_readable,
        metric_date,
        in_progress_seconds,
        toFloat64(0) AS worklog_seconds
    FROM in_progress_per_account_issue_day

    UNION ALL

    SELECT
        tenant_id,
        entity_id,
        account_source_type,
        account_source_id,
        account_id,
        insight_source_id,
        issue_id,
        data_source,
        id_readable,
        metric_date,
        toFloat64(0) AS in_progress_seconds,
        toFloat64(worklog_seconds) AS worklog_seconds
    FROM worklog_per_account_issue_day
),
person_day_rows AS (
    SELECT
        tenant_id                                                            AS day_tenant_id,
        entity_id                                                            AS day_entity_id,
        account_source_type                                                  AS day_account_source_type,
        account_source_id                                                    AS day_account_source_id,
        account_id                                                           AS day_account_id,
        {{ task_person_day_group('') }}                                      AS person_day,
        metric_date                                                          AS day_metric_date,
        insight_source_id                                                    AS day_insight_source_id,
        issue_id                                                             AS day_issue_id,
        data_source                                                          AS day_data_source,
        id_readable                                                          AS day_id_readable,
        in_progress_seconds                                                  AS day_in_progress_seconds,
        worklog_seconds                                                      AS day_worklog_seconds
    FROM account_issue_days
),
person_day_issue_rows AS (
    SELECT
        day_tenant_id,
        day_entity_id,
        person_day,
        day_metric_date,
        day_insight_source_id,
        day_issue_id,
        day_data_source,
        day_id_readable,
        day_in_progress_seconds,
        day_worklog_seconds,
        {{ task_person_day_account_columns('day_', window='person_day_window') }},
        sum(day_in_progress_seconds) OVER person_day_window                  AS person_day_in_progress_seconds
    FROM person_day_rows
    WINDOW person_day_window AS (PARTITION BY day_tenant_id, day_entity_id, person_day, day_metric_date)
)
SELECT
    assumeNotNull(r.day_tenant_id)                                           AS tenant_id,
    assumeNotNull(r.day_entity_id)                                           AS entity_id,
    any(r.account_source_type)                                               AS account_source_type,
    any(r.account_source_id)                                                 AS account_source_id,
    any(r.account_id)                                                        AS account_id,
    assumeNotNull(r.day_metric_date)                                         AS metric_date,
    assumeNotNull(r.day_insight_source_id)                                   AS insight_source_id,
    assumeNotNull(r.day_issue_id)                                            AS issue_id,
    any(r.day_data_source)                                                   AS data_source,
    -- The issue's latest key wins over the one its worklog saw, so a moved
    -- issue stays one record.
    coalesce(nullIf(any(s.id_readable), ''), any(r.day_id_readable), '')     AS id_readable,
    any(s.title)                                                             AS title,
    sum(r.day_in_progress_seconds)                                           AS in_progress_seconds,
    sum(r.day_worklog_seconds)                                               AS worklog_seconds,
    any(r.person_day_in_progress_seconds)                                    AS person_day_in_progress_seconds
FROM person_day_issue_rows AS r
LEFT JOIN {{ ref('task_issue_state') }} AS s
    ON s.insight_source_id = r.day_insight_source_id AND s.issue_id = r.day_issue_id
GROUP BY r.day_tenant_id, r.day_entity_id, r.person_day, r.day_metric_date,
         r.day_insight_source_id, r.day_issue_id
