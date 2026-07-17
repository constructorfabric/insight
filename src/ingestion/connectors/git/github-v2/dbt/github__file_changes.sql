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
    -- File extension: last segment after the final '.', empty when none.
    -- Earlier shape (issue #494) used `position('.', filename) > 0` as the
    -- guard — but ClickHouse `position` is function-style
    -- `position(haystack, needle)`, so this asked "is the string `filename`
    -- present inside the single character '.'?" — always false. Result:
    -- `file_extension` was empty for 100% of rows. Length check on the
    -- split array is more robust than a fixed `position(filename, '.') > 0`
    -- swap because it correctly returns '' for extensionless paths like
    -- `Makefile` (where the position-based guard would also fire 0 by
    -- accident, but the array-length guard is the explicit predicate).
    if(
        length(splitByChar('.', COALESCE(filename, ''))) > 1,
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
