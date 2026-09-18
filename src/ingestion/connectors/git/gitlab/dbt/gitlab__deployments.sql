-- depends_on: {{ ref('gitlab__bronze_promoted') }}
{{ config(
    materialized='incremental',
    unique_key='unique_key',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    schema='staging',
    tags=['gitlab', 'silver:class_git_deployments']
) }}

-- GitLab deployments -> the vendor-neutral deployment class. Bronze keeps one
-- row per (deployment, status); the deployment itself is the newest of them,
-- and the outcome lives in class_git_deployment_events.
--
-- is_production follows the environment tier GitLab assigns, read from the
-- environment listing: the deployment embeds its environment without the
-- tier. GitLab marks no environment as ephemeral — a review app is an
-- ordinary environment that gets stopped — so is_transient is 0 for every row.
WITH environment_tiers AS (
    SELECT
        tenant_id,
        source_id,
        project_id,
        id AS environment_id,
        tier,
        _airbyte_extracted_at
    FROM {{ source('bronze_gitlab', 'environments') }} FINAL
),

latest_status AS (
    SELECT
        tenant_id,
        source_id,
        project_id,
        id,
        repo_path,
        ref,
        sha,
        environment_id,
        environment_name,
        user_username,
        created_at,
        _airbyte_extracted_at
    FROM {{ source('bronze_gitlab', 'deployments') }} FINAL
    ORDER BY parseDateTimeBestEffortOrNull(updated_at) DESC
    LIMIT 1 BY tenant_id, source_id, project_id, id
)

SELECT
    ls.tenant_id,
    ls.source_id,
    concat(COALESCE(ls.tenant_id, ''), ':', COALESCE(ls.source_id, ''), ':', toString(COALESCE(ls.project_id, 0)), ':', toString(COALESCE(ls.id, 0))) AS unique_key,
    COALESCE(ls.repo_path, '') AS repo_full_name,
    toString(COALESCE(ls.id, 0)) AS deployment_id,
    COALESCE(ls.environment_name, '') AS environment,
    if(COALESCE(et.tier, '') = 'production', 1, 0) AS is_production,
    0 AS is_transient,
    COALESCE(ls.ref, '') AS ref,
    COALESCE(ls.sha, '') AS commit_sha,
    '' AS task,
    COALESCE(ls.user_username, '') AS creator_login,
    parseDateTimeBestEffortOrNull(ls.created_at) AS created_at,
    'insight_gitlab' AS data_source,
    toUnixTimestamp64Milli(now64()) AS _version,
    -- The tier arrives on its own stream and can change later; either side
    -- moving must re-emit the deployment row.
    greatest(
        ls._airbyte_extracted_at,
        COALESCE(et._airbyte_extracted_at, ls._airbyte_extracted_at)
    ) AS _airbyte_extracted_at
FROM latest_status AS ls
LEFT JOIN environment_tiers AS et
    ON et.tenant_id = ls.tenant_id
    AND et.source_id = ls.source_id
    AND et.project_id = ls.project_id
    AND et.environment_id = ls.environment_id
{% if is_incremental() %}
WHERE greatest(
    ls._airbyte_extracted_at,
    COALESCE(et._airbyte_extracted_at, ls._airbyte_extracted_at)
) > (SELECT max(_airbyte_extracted_at) FROM {{ this }})
{% endif %}
