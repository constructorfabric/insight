{{ config(
    materialized='incremental',
    unique_key='unique_key',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    schema='staging',
    tags=['github', 'silver:class_git_pull_requests_comments']
) }}

SELECT
    tenant_id,
    source_id,
    unique_key,
    COALESCE(repo_owner, '') AS project_key,
    COALESCE(repo_name, '') AS repo_slug,
    COALESCE(pr_database_id, 0) AS pr_id,
    COALESCE(database_id, 0) AS comment_id,
    COALESCE(body, '') AS content,
    COALESCE(author_login, '') AS author_name,
    toString(COALESCE(author_database_id, 0)) AS author_uuid,
    parseDateTimeBestEffortOrNull(created_at) AS created_at,
    parseDateTimeBestEffortOrNull(updated_at) AS updated_at,
    if(is_inline = true, 1, 0) AS is_inline,
    COALESCE(path, '') AS file_path,
    COALESCE(line, 0) AS line_number,
    'insight_github' AS data_source,
    toUnixTimestamp64Milli(now64()) AS _version,
    _airbyte_extracted_at
FROM {{ source('bronze_github', 'pull_request_comments') }}
{% if is_incremental() %}
WHERE _airbyte_extracted_at > (SELECT max(_airbyte_extracted_at) FROM {{ this }})
{% endif %}
