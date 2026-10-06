{{ config(
    tags=['connector_quality', 'youtrack'],
    meta={
        'title': 'Every field activity targets an issue with a snapshot',
        'domain': 'task-tracking',
        'category': 'completeness',
        'tier': 'warn',
        'remediation': 'The activity feed named an issue that neither issue stream returned. youtrack_activity_issues re-reads activity targets; an issue deleted or moved out of reach since the event cannot be re-read and stays listed here.'
    }
) }}

SELECT a.source_id, a.id
FROM {{ source('bronze_youtrack', 'youtrack_activities') }} AS a FINAL
LEFT ANTI JOIN {{ ref('youtrack__issues') }} AS i
    ON i.insight_source_id = a.source_id
    AND i.issue_id = JSONExtractString({{ youtrack_json('a.target_json') }}, 'id')
WHERE a._type IN ('CustomFieldActivityItem', 'TextCustomFieldActivityItem')
