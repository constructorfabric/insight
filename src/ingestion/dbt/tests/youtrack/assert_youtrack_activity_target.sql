SELECT a.source_id, a.id
FROM {{ source('bronze_youtrack', 'youtrack_activities') }} AS a FINAL
LEFT ANTI JOIN {{ ref('youtrack__issues') }} AS i
    ON i.insight_source_id = a.source_id
    AND i.issue_id = JSONExtractString({{ youtrack_json('a.target_json') }}, 'id')
WHERE a._type IN ('CustomFieldActivityItem', 'TextCustomFieldActivityItem')
