{{ config(
    tags=['connector_quality', 'youtrack'],
    meta={
        'title': 'Issue custom fields carry their native field id',
        'domain': 'task-tracking',
        'category': 'integrity',
        'tier': 'error',
        'remediation': 'A custom field arrived without projectCustomField.field.id, so it cannot be bound to a role. Check the issue stream `fields` query still requests projectCustomField(field(id)).'
    }
) }}

SELECT insight_source_id, issue_id, JSONExtractString(f, 'id') AS issue_field_id
FROM {{ ref('youtrack__issue_observations') }} FINAL
ARRAY JOIN JSONExtractArrayRaw(custom_fields) AS f
WHERE JSONExtractString(f, 'projectCustomField', 'field', 'id') = ''
