{{ config(
    materialized='table',
    engine='MergeTree',
    order_by=['insight_source_id', 'issue_id'],
    schema='insight',
    alias='task_issue_state',
    tags=['gold'],
    query_settings={
        'max_memory_usage': 1610612736,
        'max_threads': 4,
        'max_bytes_before_external_group_by': 805306368,
        'max_bytes_before_external_sort': 805306368
    }
) }}

-- Per-issue lifecycle state for the task-delivery observation pipeline: one
-- row per assignee-resolved issue carrying tenant/email attribution, the
-- current status category, the close time, and the scalar fields the
-- downstream measures read. Materialized so the field-history pivot (two
-- scans plus joins) runs exactly once per build — the observation model's
-- measure branches re-reference this state per ClickHouse's WITH inlining,
-- and re-evaluating the pivot per branch multiplied the build's reads by
-- orders of magnitude.
--
-- Lifecycle is derived from the source-neutral class_task_statuses
-- status_category ('done' = closed) joined on the status id in
-- class_task_field_history.value_ids[1]. Never match status display names.
--
-- Attribution: assignee account id resolves to a lowercased email via
-- class_task_users; only email-shaped keys pass (Jira Cloud privacy hides
-- the rest — excluded as unmatchable, not carried as dead entities). The
-- tenant rides in on the same user row.
--
-- All class reads keep FINAL: the ReplacingMergeTree parts are not
-- duplicate-immune and argMax over a stale row version would skew the pivot.

WITH
task_users AS (
    SELECT
        tenant_id,
        insight_source_id,
        user_id,
        lower(email) AS email
    FROM {{ ref('class_task_users') }} FINAL
    WHERE email LIKE '%@%'
),
-- Per-issue scalar pivot: current status id, assignee, type, due date, and the
-- estimate/spent fields, plus created (first synthetic_initial) and the last
-- status-change time (for staleness).
issue_pivot AS (
    SELECT
        insight_source_id,
        issue_id,
        argMaxIf(value_ids[1], (event_at, _version),
                 field_id = 'status' AND delta_action = 'set')               AS status_id,
        argMaxIf(value_ids[1], (event_at, _version),
                 field_id = 'assignee' AND delta_action = 'set')             AS assignee_account_id,
        argMaxIf(value_displays[1], (event_at, _version),
                 field_id = 'issuetype' AND delta_action = 'set')            AS issue_type,
        argMaxIf(value_displays[1], (event_at, _version),
                 field_id = 'duedate' AND delta_action = 'set')              AS due_date_str,
        toFloat64OrNull(argMaxIf(value_displays[1], (event_at, _version),
                 field_id = 'timeoriginalestimate' AND delta_action = 'set')) AS time_estimate_seconds,
        toFloat64OrNull(argMaxIf(value_displays[1], (event_at, _version),
                 field_id = 'timespent' AND delta_action = 'set'))           AS time_spent_seconds,
        minIf(event_at, event_kind = 'synthetic_initial')                    AS created_at,
        maxIf(event_at, field_id = 'status' AND delta_action = 'set')        AS last_status_event_at
    FROM {{ ref('class_task_field_history') }} FINAL
    WHERE field_id IN ('status', 'assignee', 'issuetype', 'duedate',
                       'timeoriginalestimate', 'timespent')
       OR event_kind = 'synthetic_initial'
    GROUP BY insight_source_id, issue_id
),
-- Close time: the last transition into a done-category status.
issue_close AS (
    SELECT
        fh.insight_source_id                                                 AS insight_source_id,
        fh.issue_id                                                          AS issue_id,
        maxIf(fh.event_at, st.status_category = 'done')                      AS final_close_at
    FROM {{ ref('class_task_field_history') }} AS fh FINAL
    LEFT JOIN {{ ref('class_task_statuses') }} AS st FINAL
        ON st.insight_source_id = fh.insight_source_id
        AND st.status_id = fh.value_ids[1]
    WHERE fh.field_id = 'status' AND fh.delta_action = 'set'
    GROUP BY fh.insight_source_id, fh.issue_id
)
SELECT
    u.tenant_id                                                              AS tenant_id,
    u.email                                                                  AS entity_id,
    p.insight_source_id                                                      AS insight_source_id,
    p.issue_id                                                               AS issue_id,
    cur.status_category                                                      AS status_category,
    p.issue_type                                                             AS issue_type,
    if(p.due_date_str IS NOT NULL AND p.due_date_str != '',
       toDate(parseDateTimeBestEffortOrNull(p.due_date_str)),
       CAST(NULL AS Nullable(Date)))                                         AS due_date,
    p.time_estimate_seconds                                                  AS time_estimate_seconds,
    p.time_spent_seconds                                                     AS time_spent_seconds,
    p.created_at                                                             AS created_at,
    c.final_close_at                                                         AS final_close_at,
    p.last_status_event_at                                                   AS last_status_event_at
FROM issue_pivot AS p
INNER JOIN task_users AS u
    ON u.insight_source_id = p.insight_source_id
    AND u.user_id = p.assignee_account_id
LEFT JOIN issue_close AS c
    ON c.insight_source_id = p.insight_source_id AND c.issue_id = p.issue_id
LEFT JOIN {{ ref('class_task_statuses') }} AS cur FINAL
    ON cur.insight_source_id = p.insight_source_id AND cur.status_id = p.status_id
