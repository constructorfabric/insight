{{ config(
    materialized='incremental',
    incremental_strategy='append',
    unique_key='unique_key',
    engine=insight_engine('ReplacingMergeTree', '_version'),
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    schema='staging',
    tags=['allure']
) }}

WITH bronze AS (
    SELECT
        b.*,
        if(JSONType(coalesce(tags, 'null')) = 'String', JSONExtractString(coalesce(tags, 'null')), coalesce(tags, 'null')) AS tags_json
    FROM {{ source('bronze_allure', 'launches') }} AS b FINAL
    {% if is_incremental() %}
    LEFT JOIN (
        SELECT tenant_id, source_id, max(_airbyte_extracted_at) AS watermark
        FROM {{ this }}
        GROUP BY tenant_id, source_id
    ) AS w
        ON w.tenant_id = b.tenant_id AND w.source_id = b.source_id
    WHERE b._airbyte_extracted_at > coalesce(w.watermark, toDateTime64(0, 3))
    {% endif %}
)

SELECT
    tenant_id,
    source_id,
    unique_key,
    toInt64(COALESCE(id, 0)) AS launch_id,
    toInt64(COALESCE(projectId, 0)) AS project_id,
    COALESCE(name, '') AS launch_name,
    COALESCE(closed, false) AS is_closed,
    COALESCE(external, false) AS is_external,
    arrayFilter(n -> n != '', arrayMap(t -> JSONExtractString(t, 'name'), JSONExtractArrayRaw(tags_json))) AS tag_names,
    COALESCE(createdBy, '') AS created_by,
    fromUnixTimestamp64Milli(createdDate, 'UTC') AS created_at,
    fromUnixTimestamp64Milli(lastModifiedDate, 'UTC') AS last_modified_at,
    'insight_allure' AS data_source,
    toUnixTimestamp64Milli(now64()) AS _version,
    _airbyte_extracted_at
FROM bronze
