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
person_days AS (
    SELECT
        day_tenant_id,
        day_entity_id,
        person_day,
        day_metric_date,
        {{ task_person_day_account_columns('day_') }},
        sum(day_in_progress_seconds)                                         AS person_day_in_progress_seconds
    FROM {{ ref('task_person_day_rows') }}
    GROUP BY day_tenant_id, day_entity_id, person_day, day_metric_date
),
person_day_issue_rows AS (
    SELECT
        day_tenant_id,
        day_entity_id,
        person_day,
        day_metric_date,
        day_insight_source_id,
        day_issue_id,
        any(day_data_source)                                                 AS data_source,
        any(day_id_readable)                                                 AS id_readable,
        sum(day_in_progress_seconds)                                         AS in_progress_seconds,
        sum(day_worklog_seconds)                                             AS worklog_seconds
    FROM {{ ref('task_person_day_rows') }}
    GROUP BY day_tenant_id, day_entity_id, person_day, day_metric_date,
             day_insight_source_id, day_issue_id
)
SELECT
    assumeNotNull(r.day_tenant_id)                                           AS tenant_id,
    assumeNotNull(r.day_entity_id)                                           AS entity_id,
    d.account_source_type                                                    AS account_source_type,
    d.account_source_id                                                      AS account_source_id,
    d.account_id                                                             AS account_id,
    assumeNotNull(r.day_metric_date)                                         AS metric_date,
    assumeNotNull(r.day_insight_source_id)                                   AS insight_source_id,
    assumeNotNull(r.day_issue_id)                                            AS issue_id,
    r.data_source                                                            AS data_source,
    -- The issue's latest key wins over the one its worklog saw, so a moved
    -- issue stays one record.
    coalesce(nullIf(s.id_readable, ''), r.id_readable, '')                   AS id_readable,
    s.title                                                                  AS title,
    r.in_progress_seconds                                                    AS in_progress_seconds,
    r.worklog_seconds                                                        AS worklog_seconds,
    d.person_day_in_progress_seconds                                         AS person_day_in_progress_seconds
FROM person_day_issue_rows AS r
INNER JOIN person_days AS d
    ON d.day_tenant_id IS NOT DISTINCT FROM r.day_tenant_id
    AND d.day_entity_id = r.day_entity_id
    AND d.person_day = r.person_day
    AND d.day_metric_date = r.day_metric_date
LEFT JOIN {{ ref('task_issue_state') }} AS s
    ON s.insight_source_id = r.day_insight_source_id AND s.issue_id = r.day_issue_id
