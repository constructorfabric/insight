{{ config(
    materialized='table',
    engine='ReplacingMergeTree',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    schema='staging',
    tags=['allure']
) }}

WITH bronze AS (
    SELECT
        tenant_id,
        source_id,
        unique_key AS test_case_unique_key,
        toInt64(COALESCE(id, 0)) AS test_case_id,
        toInt64(COALESCE(projectId, 0)) AS project_id,
        COALESCE(deleted, false) AS is_test_case_deleted,
        if(JSONType(coalesce(customFields, 'null')) = 'String', JSONExtractString(coalesce(customFields, 'null')), coalesce(customFields, 'null')) AS custom_fields_json
    FROM {{ source('bronze_allure', 'test_cases') }} FINAL
)

SELECT
    tenant_id,
    source_id,
    concat(test_case_unique_key, '-', toString(JSONExtractInt(v, 'id'))) AS unique_key,
    test_case_id,
    project_id,
    JSONExtractInt(v, 'customField', 'id') AS field_id,
    JSONExtractString(v, 'customField', 'name') AS field_name,
    JSONExtractInt(v, 'id') AS value_id,
    JSONExtractString(v, 'name') AS value,
    is_test_case_deleted,
    'insight_allure' AS data_source
FROM bronze
ARRAY JOIN JSONExtractArrayRaw(custom_fields_json) AS v
