SELECT insight_source_id, issue_id, JSONExtractString(f, 'id') AS issue_field_id
FROM {{ ref('youtrack__issue_observations') }} FINAL
ARRAY JOIN JSONExtractArrayRaw(custom_fields) AS f
WHERE JSONExtractString(f, 'projectCustomField', 'field', 'id') = ''
