{{ config(
    materialized='incremental',
    unique_key='unique_key',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    schema='staging',
    tags=['github', 'silver:class_git_file_changes']
) }}

SELECT
    tenant_id,
    source_id,
    unique_key,
    COALESCE(repo_owner, '') AS project_key,
    COALESCE(repo_name, '') AS repo_slug,
    COALESCE(commit_hash, '') AS commit_hash,
    COALESCE(filename, '') AS file_path,
    if(
        position('.', COALESCE(filename, '')) > 0,
        arrayElement(splitByChar('.', COALESCE(filename, '')), -1),
        ''
    ) AS file_extension,
    COALESCE(status, '') AS change_type,
    COALESCE(additions, 0) AS lines_added,
    COALESCE(deletions, 0) AS lines_removed,
    COALESCE(source_type, '') AS source_type,
    'insight_github' AS data_source,
    toUnixTimestamp64Milli(now64()) AS _version,
    _airbyte_extracted_at
FROM {{ source('bronze_github', 'file_changes') }}
{% if is_incremental() %}
WHERE _airbyte_extracted_at > (SELECT max(_airbyte_extracted_at) FROM {{ this }})
{% endif %}
