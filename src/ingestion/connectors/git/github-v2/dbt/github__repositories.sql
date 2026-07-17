{{ config(
    materialized='incremental',
    unique_key='unique_key',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    schema='staging',
    tags=['github', 'silver:class_git_repositories']
) }}

SELECT
    tenant_id,
    source_id,
    unique_key,
    COALESCE(JSONExtractString(owner, 'login'), '') AS project_key,
    COALESCE(name, '') AS repo_slug,
    toString(COALESCE(id, 0)) AS repo_uuid,
    COALESCE(name, '') AS name,
    COALESCE(full_name, '') AS full_name,
    COALESCE(description, '') AS description,
    if(private = true, 1, 0) AS is_private,
    parseDateTimeBestEffortOrNull(created_at) AS created_on,
    parseDateTimeBestEffortOrNull(COALESCE(pushed_at, updated_at)) AS updated_on,
    COALESCE(size, 0) AS size,
    COALESCE(language, '') AS language,
    if(has_issues = true, 1, 0) AS has_issues,
    if(has_wiki = true, 1, 0) AS has_wiki,
    '' AS metadata,
    'insight_github' AS data_source,
    toUnixTimestamp64Milli(now64()) AS _version,
    _airbyte_extracted_at
FROM {{ source('bronze_github', 'repositories') }}
{% if is_incremental() %}
WHERE _airbyte_extracted_at > (SELECT max(_airbyte_extracted_at) FROM {{ this }})
{% endif %}
