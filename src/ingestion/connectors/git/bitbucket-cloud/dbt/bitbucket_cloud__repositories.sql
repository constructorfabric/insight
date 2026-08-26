-- depends_on: {{ ref('bitbucket_cloud__bronze_promoted') }}
{{ config(
    materialized='incremental',
    unique_key='unique_key',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    schema='staging',
    tags=['bitbucket-cloud', 'silver:class_git_repositories']
) }}

-- FINAL: a repository is mutable, so a re-fetch within one sync can leave a
-- pre-merge duplicate in bronze. Both copies would take this run's `_version`
-- and tie the class dedup, letting the stale one win.
SELECT
    tenant_id,
    source_id,
    unique_key,
    splitByChar('/', COALESCE(full_name, ''))[1] AS project_key,
    COALESCE(slug, '') AS repo_slug,
    COALESCE(repository_uuid, '') AS repo_uuid,
    COALESCE(name, '') AS name,
    COALESCE(full_name, '') AS full_name,
    COALESCE(description, '') AS description,
    if(COALESCE(is_private, false), 1, 0) AS is_private,
    parseDateTimeBestEffortOrNull(created_on) AS created_on,
    parseDateTimeBestEffortOrNull(updated_on) AS updated_on,
    COALESCE(size, 0) AS size,
    COALESCE(language, '') AS language,
    if(COALESCE(has_issues, false), 1, 0) AS has_issues,
    if(COALESCE(has_wiki, false), 1, 0) AS has_wiki,
    '' AS metadata,
    'insight_bitbucket_cloud' AS data_source,
    toUnixTimestamp64Milli(now64()) AS _version,
    _airbyte_extracted_at
FROM {{ source('bronze_bitbucket_cloud', 'repositories') }} FINAL
{% if is_incremental() %}
WHERE _airbyte_extracted_at > (SELECT max(_airbyte_extracted_at) FROM {{ this }})
{% endif %}
