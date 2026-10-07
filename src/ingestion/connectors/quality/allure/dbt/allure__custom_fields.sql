{{ config(
    materialized='table',
    engine='ReplacingMergeTree',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    schema='staging',
    tags=['allure']
) }}

-- Object columns land as String or JSON depending on the installation; toString reads both.
SELECT
    tenant_id,
    source_id,
    unique_key,
    toInt64(COALESCE(projectId, 0)) AS project_id,
    toInt64(COALESCE(id, 0)) AS field_id,
    COALESCE(name, '') AS field_name,
    COALESCE(required, false) AS is_required,
    COALESCE(singleSelect, false) AS is_single_select,
    COALESCE(locked, false) AS is_locked,
    toBool(JSONExtractBool(
        if(JSONType(coalesce(toString(customField), 'null')) = 'String', JSONExtractString(coalesce(toString(customField), 'null')), coalesce(toString(customField), 'null')),
        'archived'
    )) AS is_archived,
    'insight_allure' AS data_source
FROM {{ source('bronze_allure', 'custom_fields') }} FINAL
