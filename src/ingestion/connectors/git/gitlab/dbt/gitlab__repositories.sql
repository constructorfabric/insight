-- depends_on: {{ ref('gitlab__bronze_promoted') }}
{{ config(
    materialized='incremental',
    unique_key='unique_key',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    schema='staging',
    tags=['gitlab', 'silver:class_git_repositories']
) }}

-- GitLab project -> class_git_repositories. A GitLab project IS the repo;
-- its namespace (group path) maps to project_key, its path to repo_slug.
-- language / has_issues / has_wiki are not collected by the connector's
-- projects stream; emitted as defaults until those fields are added.
SELECT
    tenant_id,
    source_id,
    unique_key,
    COALESCE(namespace_full_path, '') AS project_key,
    COALESCE(path, '') AS repo_slug,
    toString(COALESCE(id, 0)) AS repo_uuid,
    COALESCE(name, '') AS name,
    COALESCE(path_with_namespace, '') AS full_name,
    COALESCE(description, '') AS description,
    if(visibility = 'public', 0, 1) AS is_private,
    parseDateTimeBestEffortOrNull(created_at) AS created_on,
    parseDateTimeBestEffortOrNull(last_activity_at) AS updated_on,
    COALESCE(statistics_repository_size, 0) AS size,
    '' AS language,
    0 AS has_issues,
    0 AS has_wiki,
    '' AS metadata,
    'insight_gitlab' AS data_source,
    toUnixTimestamp64Milli(now64()) AS _version,
    _airbyte_extracted_at
FROM {{ source('bronze_gitlab', 'projects') }} FINAL
{% if is_incremental() %}
WHERE _airbyte_extracted_at > (SELECT max(_airbyte_extracted_at) FROM {{ this }})
{% endif %}
