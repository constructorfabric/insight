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
        if(JSONType(coalesce(toString(variable), 'null')) = 'String', JSONExtractString(coalesce(toString(variable), 'null')), coalesce(toString(variable), 'null')) AS variable_json
    FROM {{ source('bronze_allure', 'launch_environment') }} AS b FINAL
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
    toInt64(COALESCE(launch_id, 0)) AS launch_id,
    toInt64(COALESCE(project_id, 0)) AS project_id,
    toInt64(COALESCE(id, 0)) AS value_id,
    JSONExtractString(variable_json, 'name') AS variable_name,
    COALESCE(name, '') AS value,
    fromUnixTimestamp64Milli(lastModifiedDate, 'UTC') AS launch_last_modified_at,
    'insight_allure' AS data_source,
    toUnixTimestamp64Milli(now64()) AS _version,
    _airbyte_extracted_at
FROM bronze
