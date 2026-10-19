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

-- Object columns land as String or JSON depending on the installation; toString reads both.
WITH bronze AS (
    SELECT
        b.*,
        if(JSONType(coalesce(tags, 'null')) = 'String', JSONExtractString(coalesce(tags, 'null')), coalesce(tags, 'null')) AS tags_json,
        if(JSONType(coalesce(toString(layer), 'null')) = 'String', JSONExtractString(coalesce(toString(layer), 'null')), coalesce(toString(layer), 'null')) AS layer_json,
        if(JSONType(coalesce(toString(jobRun), 'null')) = 'String', JSONExtractString(coalesce(toString(jobRun), 'null')), coalesce(toString(jobRun), 'null')) AS job_run_json,
        if(JSONType(coalesce(toString(category), 'null')) = 'String', JSONExtractString(coalesce(toString(category), 'null')), coalesce(toString(category), 'null')) AS category_json
    FROM {{ source('bronze_allure', 'test_results') }} AS b FINAL
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

env_values AS (
    SELECT
        tenant_id,
        source_id,
        toInt64(COALESCE(launch_id, 0)) AS launch_id,
        JSONExtractString(if(JSONType(coalesce(toString(variable), 'null')) = 'String', JSONExtractString(coalesce(toString(variable), 'null')), coalesce(toString(variable), 'null')), 'name') AS variable_name,
        argMax(COALESCE(name, ''), id) AS value
    FROM {{ source('bronze_allure', 'launch_environment') }} FINAL
    GROUP BY tenant_id, source_id, launch_id, variable_name
),

env AS (
    SELECT
        tenant_id AS env_tenant_id,
        source_id AS env_source_id,
        launch_id AS env_launch_id,
        toJSONString(mapFromArrays(groupArray(variable_name), groupArray(value))) AS env_json
    FROM env_values
    GROUP BY tenant_id, source_id, launch_id
),

instance AS (
    SELECT
        tenant_id AS instance_tenant_id,
        source_id AS instance_source_id,
        toInt64(COALESCE(id, 0)) AS instance_project_id,
        COALESCE(allure_url, '') AS base_url
    FROM {{ source('bronze_allure', 'projects') }} FINAL
    WHERE COALESCE(allure_url, '') != ''
)

SELECT
    tenant_id,
    source_id,
    unique_key,
    toInt64(COALESCE(id, 0)) AS test_result_id,
    toInt64(COALESCE(launchId, 0)) AS launch_id,
    toInt64(COALESCE(projectId, 0)) AS project_id,
    CAST(testCaseId AS Nullable(Int64)) AS test_case_id,
    COALESCE(name, '') AS test_name,
    COALESCE(fullName, '') AS full_name,
    COALESCE(status, '') AS status,
    CAST(duration AS Nullable(Int64)) AS duration_ms,
    fromUnixTimestamp64Milli(start, 'UTC') AS started_at,
    fromUnixTimestamp64Milli(stop, 'UTC') AS stopped_at,
    COALESCE(hidden, false) AS is_hidden,
    COALESCE(flaky, false) AS is_flaky,
    COALESCE(muted, false) AS is_muted,
    COALESCE(manual, false) AS is_manual,
    COALESCE(known, false) AS is_known,
    JSONExtractString(layer_json, 'name') AS layer_name,
    JSONExtractString(job_run_json, 'name') AS job_run_name,
    JSONExtractString(job_run_json, 'url') AS job_run_url,
    JSONExtract(category_json, 'id', 'Nullable(Int64)') AS category_id,
    JSONExtractString(category_json, 'name') AS category_name,
    COALESCE(assignee, '') AS assignee,
    COALESCE(testedBy, '') AS tested_by,
    COALESCE(historyKey, '') AS history_key,
    COALESCE(message, '') AS message,
    ifNull(nullIf(env_json, ''), '{}') AS launch_env,
    if(ifNull(base_url, '') = '', '', concat(ifNull(base_url, ''), '/project/', toString(COALESCE(projectId, 0)), '/test-results/', toString(COALESCE(id, 0)))) AS test_result_url,
    if(ifNull(base_url, '') = '', '', concat(ifNull(base_url, ''), '/project/', toString(COALESCE(projectId, 0)), '/launches/', toString(COALESCE(launchId, 0)))) AS launch_url,
    if(ifNull(base_url, '') = '' OR testCaseId IS NULL, '', concat(ifNull(base_url, ''), '/project/', toString(COALESCE(projectId, 0)), '/test-cases/', toString(ifNull(testCaseId, 0)))) AS test_case_url,
    arrayFilter(n -> n != '', arrayMap(t -> JSONExtractString(t, 'name'), JSONExtractArrayRaw(tags_json))) AS tag_names,
    fromUnixTimestamp64Milli(createdDate, 'UTC') AS created_at,
    fromUnixTimestamp64Milli(lastModifiedDate, 'UTC') AS last_modified_at,
    'insight_allure' AS data_source,
    toUnixTimestamp64Milli(now64()) AS _version,
    _airbyte_extracted_at
FROM bronze
LEFT JOIN env
    ON env.env_tenant_id = bronze.tenant_id
    AND env.env_source_id = bronze.source_id
    AND env.env_launch_id = toInt64(COALESCE(bronze.launchId, 0))
LEFT JOIN instance
    ON instance.instance_tenant_id = bronze.tenant_id
    AND instance.instance_source_id = bronze.source_id
    AND instance.instance_project_id = toInt64(COALESCE(bronze.projectId, 0))
