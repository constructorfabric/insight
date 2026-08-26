-- depends_on: {{ ref('bitbucket_cloud__bronze_promoted') }}
{{ config(
    materialized='incremental',
    unique_key='unique_key',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    schema='staging',
    tags=['bitbucket-cloud', 'silver:class_git_repository_branches']
) }}

-- FINAL: a branch head moves, so a re-fetch within one sync can leave a
-- pre-merge duplicate in bronze that ties the class dedup on `_version`.
SELECT
    tenant_id,
    source_id,
    unique_key,
    arrayElement(splitByChar('/', COALESCE(repository, '')), -2) AS project_key,
    replaceRegexpOne(arrayElement(splitByChar('/', COALESCE(repository, '')), -1), '\\.git$', '') AS repo_slug,
    COALESCE(name, '') AS branch_name,
    if(COALESCE(is_default, false), 1, 0) AS is_default,
    COALESCE(head_sha, '') AS last_commit_hash,
    parseDateTimeBestEffortOrNull(head_committed_date) AS last_commit_date,
    'insight_bitbucket_cloud' AS data_source,
    toUnixTimestamp64Milli(now64()) AS _version,
    _airbyte_extracted_at
FROM {{ source('bronze_bitbucket_cloud', 'branches') }} FINAL
{% if is_incremental() %}
WHERE _airbyte_extracted_at > (SELECT max(_airbyte_extracted_at) FROM {{ this }})
{% endif %}
