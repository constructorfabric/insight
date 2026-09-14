-- depends_on: {{ ref('gitlab__bronze_promoted') }}
{{ config(
    materialized='incremental',
    unique_key='unique_key',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    schema='staging',
    tags=['gitlab', 'silver:class_git_repository_branches']
) }}

-- repo_path is path_with_namespace; everything before the last segment is the
-- namespace, the last segment the project.
SELECT
    tenant_id,
    source_id,
    unique_key,
    arrayStringConcat(arrayPopBack(splitByChar('/', COALESCE(repo_path, ''))), '/') AS project_key,
    arrayElement(splitByChar('/', COALESCE(repo_path, '')), -1) AS repo_slug,
    COALESCE(name, '') AS branch_name,
    if(COALESCE(is_default, false), 1, 0) AS is_default,
    COALESCE(head_sha, '') AS last_commit_hash,
    parseDateTimeBestEffortOrNull(head_committed_date) AS last_commit_date,
    'insight_gitlab' AS data_source,
    toUnixTimestamp64Milli(now64()) AS _version,
    _airbyte_extracted_at
FROM {{ source('bronze_gitlab', 'branches') }} FINAL
{% if is_incremental() %}
WHERE _airbyte_extracted_at > (SELECT max(_airbyte_extracted_at) FROM {{ this }})
{% endif %}
