-- depends_on: {{ ref('github__bronze_promoted') }}
{{ config(
    materialized='incremental',
    unique_key='unique_key',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    schema='staging',
    tags=['github', 'silver:class_git_repositories']
) }}

-- has_issues / has_wiki are not collected and no consumer reads them, so the
-- contract columns stay at 0 rather than widening the stream for nothing.
SELECT
    tenant_id,
    source_id,
    unique_key,
    COALESCE(org, '') AS project_key,
    COALESCE(name, '') AS repo_slug,
    COALESCE(full_name, '') AS repo_uuid,
    COALESCE(name, '') AS name,
    COALESCE(full_name, '') AS full_name,
    COALESCE(description, '') AS description,
    if(COALESCE(private, false), 1, 0) AS is_private,
    parseDateTimeBestEffortOrNull(created_at) AS created_on,
    parseDateTimeBestEffortOrNull(COALESCE(pushed_at, updated_at)) AS updated_on,
    COALESCE(size, 0) AS size,
    COALESCE(language, '') AS language,
    0 AS has_issues,
    0 AS has_wiki,
    '' AS metadata,
    'insight_github' AS data_source,
    toUnixTimestamp64Milli(now64()) AS _version,
    _airbyte_extracted_at
FROM {{ source('bronze_github', 'repositories') }} FINAL
{% if is_incremental() %}
WHERE _airbyte_extracted_at > (SELECT max(_airbyte_extracted_at) FROM {{ this }})
{% endif %}
