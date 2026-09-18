-- depends_on: {{ ref('gitlab__bronze_promoted') }}
{{ config(
    materialized='incremental',
    unique_key='unique_key',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    schema='staging',
    tags=['gitlab', 'silver:class_git_file_changes']
) }}

SELECT
    tenant_id,
    source_id,
    unique_key,
    arrayStringConcat(arrayPopBack(splitByChar('/', COALESCE(repo_path, ''))), '/') AS project_key,
    arrayElement(splitByChar('/', COALESCE(repo_path, '')), -1) AS repo_slug,
    COALESCE(sha, '') AS commit_hash,
    COALESCE(filename, '') AS file_path,
    {{ git_file_extension("COALESCE(filename, '')") }} AS file_extension,
    COALESCE(status, '') AS change_type,
    toNullable(COALESCE(additions, 0)) AS lines_added,
    toNullable(COALESCE(deletions, 0)) AS lines_removed,
    -- The proxy walks commits, so every file change is reached through one.
    'commit' AS source_type,
    'insight_gitlab' AS data_source,
    toUnixTimestamp64Milli(now64()) AS _version,
    _airbyte_extracted_at,
    pre_image_oid,
    post_image_oid
FROM {{ source('bronze_gitlab', 'file_changes') }}
{% if is_incremental() %}
WHERE _airbyte_extracted_at > (SELECT max(_airbyte_extracted_at) FROM {{ this }})
{% endif %}
