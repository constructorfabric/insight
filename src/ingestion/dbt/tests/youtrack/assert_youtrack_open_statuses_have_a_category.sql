{{ config(
    severity='warn',
    tags=['connector_quality', 'youtrack'],
    meta={
        'title': 'Open YouTrack statuses in use have a category',
        'domain': 'task-tracking',
        'category': 'configuration',
        'tier': 'warn',
        'remediation': 'An open status some issue sits in has no operator decision between `new` and `in_progress`. Decide it in the environment\'s clickhouse/field-value-map/task-values.tsv.'
    }
) }}

-- An open status value that some issue sits in, with no operator decision
-- between `new` and `in_progress`. `done` comes from YouTrack's own isResolved;
-- the open split does not, and until it is decided the value is `undefined`,
-- so in-progress measures do not count its issues. Decide it in the
-- environment's clickhouse/field-value-map/task-values.tsv.
SELECT s.insight_source_id, s.status_id, any(s.status_name) AS status_name, count() AS issues
FROM {{ ref('youtrack__task_statuses') }} AS s FINAL
INNER JOIN (
    SELECT h.insight_source_id AS insight_source_id, h.issue_id AS issue_id,
        argMax(h.value_ids[1], h.event_order) AS status_id
    FROM {{ ref('youtrack__task_field_history') }} AS h FINAL
    INNER JOIN {{ ref('task_field_roles_current') }} AS r
        ON r.insight_source_id = h.insight_source_id AND r.data_source = h.data_source AND r.field_id = h.field_id
    WHERE r.role = 'status'
    GROUP BY insight_source_id, issue_id
) AS current ON current.insight_source_id = s.insight_source_id AND current.status_id = s.status_id
WHERE s.status_category = 'undefined'
GROUP BY s.insight_source_id, s.status_id
