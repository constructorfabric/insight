{{ config(
    tags=['connector_quality', 'youtrack'],
    meta={
        'title': 'Issue snapshots hold a JSON object and a custom field array',
        'domain': 'task-tracking',
        'category': 'integrity',
        'tier': 'error',
        'remediation': 'The issue payload or custom fields are not the expected JSON shape. Check how the destination stored issue_json and custom_fields_json for the listed issues.'
    }
) }}

SELECT insight_source_id, issue_id
FROM {{ ref('youtrack__issues') }}
WHERE NOT isValidJSON(payload) OR JSONType(payload) != 'Object'
   OR NOT isValidJSON(custom_fields) OR JSONType(custom_fields) != 'Array'
