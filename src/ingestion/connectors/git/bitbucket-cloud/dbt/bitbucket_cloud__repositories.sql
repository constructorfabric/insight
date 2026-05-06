-- depends_on: {{ ref('bitbucket_cloud__bronze_promoted') }}
{{ config(
    materialized='incremental',
    unique_key='unique_key',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    schema='staging',
    tags=['bitbucket-cloud', 'silver:class_git_repositories']
) }}

SELECT
    tenant_id,
    source_id,
    unique_key,
    COALESCE(workspace, '') AS project_key,
    COALESCE(slug, '') AS repo_slug,
    COALESCE(uuid, '') AS repo_uuid,
    COALESCE(name, '') AS name,
    COALESCE(full_name, '') AS full_name,
    COALESCE(description, '') AS description,
    if(is_private = true, 1, 0) AS is_private,
    parseDateTimeBestEffortOrNull(created_on) AS created_on,
    parseDateTimeBestEffortOrNull(updated_on) AS updated_on,
    COALESCE(size, 0) AS size,
    COALESCE(language, '') AS language,
    if(has_issues = true, 1, 0) AS has_issues,
    if(has_wiki = true, 1, 0) AS has_wiki,
    '' AS metadata,
    'insight_bitbucket_cloud' AS data_source,
    toUnixTimestamp64Milli(now64()) AS _version,
    _airbyte_extracted_at
FROM {{ source('bronze_bitbucket_cloud', 'repositories') }}
{% if is_incremental() %}
WHERE _airbyte_extracted_at > (SELECT max(_airbyte_extracted_at) FROM {{ this }})
{% endif %}
