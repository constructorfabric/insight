{{ config(
    materialized='incremental',
    incremental_strategy='append',
    unique_key='unique_key',
    engine='ReplacingMergeTree(_version)',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    schema='staging',
    tags=['allure']
) }}

WITH bronze AS (
    SELECT
        b.*,
        if(JSONType(coalesce(customFields, 'null')) = 'String', JSONExtractString(coalesce(customFields, 'null')), coalesce(customFields, 'null')) AS custom_fields_json,
        if(JSONType(coalesce(tags, 'null')) = 'String', JSONExtractString(coalesce(tags, 'null')), coalesce(tags, 'null')) AS tags_json,
        if(JSONType(coalesce(status, 'null')) = 'String', JSONExtractString(coalesce(status, 'null')), coalesce(status, 'null')) AS status_json,
        if(JSONType(coalesce(workflow, 'null')) = 'String', JSONExtractString(coalesce(workflow, 'null')), coalesce(workflow, 'null')) AS workflow_json,
        if(JSONType(coalesce(layer, 'null')) = 'String', JSONExtractString(coalesce(layer, 'null')), coalesce(layer, 'null')) AS layer_json
    FROM {{ source('bronze_allure', 'test_cases') }} AS b FINAL
    {% if is_incremental() %}
    LEFT JOIN (
        SELECT tenant_id, source_id, max(_airbyte_extracted_at) AS watermark
        FROM {{ this }}
        GROUP BY tenant_id, source_id
    ) AS w
        ON w.tenant_id = b.tenant_id AND w.source_id = b.source_id
    WHERE b._airbyte_extracted_at > coalesce(w.watermark, toDateTime64(0, 3))
    {% endif %}
),

pairs AS (
    SELECT
        *,
        arrayMap(
            v -> (JSONExtractString(v, 'customField', 'name'), JSONExtractString(v, 'name')),
            JSONExtractArrayRaw(custom_fields_json)
        ) AS field_value_pairs
    FROM bronze
)

SELECT
    tenant_id,
    source_id,
    unique_key,
    toInt64(COALESCE(id, 0)) AS test_case_id,
    toInt64(COALESCE(projectId, 0)) AS project_id,
    COALESCE(name, '') AS test_case_name,
    COALESCE(fullName, '') AS full_name,
    COALESCE(automated, false) AS is_automated,
    COALESCE(deleted, false) AS is_deleted,
    COALESCE(flaky, false) AS is_flaky,
    COALESCE(external, false) AS is_external,
    COALESCE(hasManualScenario, false) AS has_manual_scenario,
    COALESCE(style, '') AS style,
    JSONExtractString(status_json, 'name') AS status_name,
    JSONExtractString(workflow_json, 'name') AS workflow_name,
    JSONExtractString(layer_json, 'name') AS layer_name,
    arrayFilter(n -> n != '', arrayMap(t -> JSONExtractString(t, 'name'), JSONExtractArrayRaw(tags_json))) AS tag_names,
    CAST(
        mapFromArrays(
            arrayDistinct(arrayMap(p -> p.1, field_value_pairs)),
            arrayMap(
                f -> arrayMap(p -> p.2, arrayFilter(p -> p.1 = f, field_value_pairs)),
                arrayDistinct(arrayMap(p -> p.1, field_value_pairs))
            )
        ),
        'Map(String, Array(String))'
    ) AS custom_fields,
    COALESCE(createdBy, '') AS created_by,
    fromUnixTimestamp64Milli(createdDate, 'UTC') AS created_at,
    fromUnixTimestamp64Milli(lastModifiedDate, 'UTC') AS last_modified_at,
    'insight_allure' AS data_source,
    toUnixTimestamp64Milli(now64()) AS _version,
    _airbyte_extracted_at
FROM pairs
