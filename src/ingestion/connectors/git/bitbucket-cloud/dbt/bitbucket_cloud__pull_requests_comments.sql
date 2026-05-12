-- depends_on: {{ ref('bitbucket_cloud__bronze_promoted') }}
{{ config(
    materialized='incremental',
    unique_key='unique_key',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    schema='staging',
    tags=['bitbucket-cloud', 'silver:class_git_pull_requests_comments']
) }}

SELECT
    tenant_id,
    source_id,
    unique_key,
    COALESCE(workspace, '') AS project_key,
    COALESCE(repo_slug, '') AS repo_slug,
    COALESCE(pr_id, 0) AS pr_id,
    COALESCE(comment_id, 0) AS comment_id,
    COALESCE(body, '') AS content,
    COALESCE(author_display_name, '') AS author_name,
    COALESCE(author_uuid, '') AS author_uuid,
    parseDateTimeBestEffortOrNull(created_on) AS created_at,
    parseDateTimeBestEffortOrNull(updated_on) AS updated_at,
    if(is_inline = true, 1, 0) AS is_inline,
    COALESCE(inline_path, '') AS file_path,
    COALESCE(inline_to, COALESCE(inline_from, 0)) AS line_number,
    'insight_bitbucket_cloud' AS data_source,
    toUnixTimestamp64Milli(now64()) AS _version,
    _airbyte_extracted_at
FROM {{ source('bronze_bitbucket_cloud', 'pull_request_comments') }}
{% if is_incremental() %}
WHERE _airbyte_extracted_at > (SELECT max(_airbyte_extracted_at) FROM {{ this }})
{% endif %}
