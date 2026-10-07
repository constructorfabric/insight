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
    SELECT b.*
    FROM {{ source('bronze_allure', 'launch_errors') }} AS b FINAL
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
    toInt64(COALESCE(id, 0)) AS launch_error_id,
    toInt64(COALESCE(launch_id, 0)) AS launch_id,
    toInt64(COALESCE(project_id, 0)) AS project_id,
    CAST(jobRunId AS Nullable(Int64)) AS job_run_id,
    COALESCE(hash, '') AS error_hash,
    COALESCE(message, '') AS message,
    COALESCE(trace, '') AS trace,
    fromUnixTimestamp64Milli(lastModifiedDate, 'UTC') AS launch_last_modified_at,
    'insight_allure' AS data_source,
    toUnixTimestamp64Milli(now64()) AS _version,
    _airbyte_extracted_at
FROM bronze
