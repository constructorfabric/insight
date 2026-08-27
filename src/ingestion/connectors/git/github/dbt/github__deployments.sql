-- depends_on: {{ ref('github__bronze_promoted') }}
{{ config(
    materialized='incremental',
    unique_key='unique_key',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    schema='staging',
    tags=['github', 'silver:class_git_deployments']
) }}

-- GitHub deployments -> the vendor-neutral deployment class. A deployment
-- record carries NO outcome — that arrives as status events in
-- class_git_deployment_events, and readers fold the latest one in. A
-- deployment with no event yet is pending, and pending must stay visible.
--
-- FINAL: the record is mutable (updated_at moves), so a window re-fetch
-- within one sync can leave a pre-merge duplicate in bronze.
SELECT
    tenant_id,
    source_id,
    unique_key,
    COALESCE(repo_full_name, '') AS repo_full_name,
    toString(COALESCE(id, 0)) AS deployment_id,
    COALESCE(environment, '') AS environment,
    if(COALESCE(is_production_environment, false), 1, 0) AS is_production,
    if(COALESCE(is_transient_environment, false), 1, 0) AS is_transient,
    COALESCE(ref, '') AS ref,
    COALESCE(sha, '') AS commit_sha,
    COALESCE(task, '') AS task,
    COALESCE(creator_login, '') AS creator_login,
    parseDateTimeBestEffortOrNull(created_at) AS created_at,
    'insight_github' AS data_source,
    toUnixTimestamp64Milli(now64()) AS _version,
    _airbyte_extracted_at
FROM {{ source('bronze_github', 'deployments') }} FINAL
{% if is_incremental() %}
WHERE _airbyte_extracted_at > (SELECT max(_airbyte_extracted_at) FROM {{ this }})
{% endif %}
