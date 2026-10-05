SELECT insight_source_id, issue_id
FROM {{ ref('youtrack__issues') }}
WHERE NOT isValidJSON(payload) OR JSONType(payload) != 'Object'
   OR NOT isValidJSON(custom_fields) OR JSONType(custom_fields) != 'Array'
