-- depends_on: {{ ref('gitlab__bronze_promoted') }}
{{ config(
    materialized='incremental',
    unique_key='unique_key',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    schema='staging',
    tags=['gitlab', 'silver:class_git_repositories']
) }}

-- project_key is the full namespace path (group and subgroups), repo_slug the
-- project's own path segment: together they are path_with_namespace, which is
-- how every other GitLab model names the project.
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
    if(COALESCE(visibility, '') != 'public', 1, 0) AS is_private,
    parseDateTimeBestEffortOrNull(created_at) AS created_on,
    parseDateTimeBestEffortOrNull(last_activity_at) AS updated_on,
    COALESCE(repository_size, 0) AS size,
    '' AS language,
    if(COALESCE(issues_enabled, false), 1, 0) AS has_issues,
    if(COALESCE(wiki_enabled, false), 1, 0) AS has_wiki,
    '' AS metadata,
    'insight_gitlab' AS data_source,
    toUnixTimestamp64Milli(now64()) AS _version,
    _airbyte_extracted_at,
    toNullable(default_branch) AS default_branch
FROM {{ source('bronze_gitlab', 'repositories') }} FINAL
{% if is_incremental() %}
WHERE _airbyte_extracted_at > (SELECT max(_airbyte_extracted_at) FROM {{ this }})
{% endif %}
