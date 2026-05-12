-- depends_on: {{ ref('bitbucket_cloud__bronze_promoted') }}
{{ config(
    materialized='incremental',
    unique_key='unique_key',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    schema='staging',
    tags=['bitbucket-cloud', 'silver:class_git_pull_requests_commits']
) }}

SELECT
    tenant_id,
    source_id,
    unique_key,
    COALESCE(workspace, '') AS project_key,
    COALESCE(repo_slug, '') AS repo_slug,
    COALESCE(pr_id, 0) AS pr_id,
    COALESCE(hash, '') AS commit_hash,
    0 AS commit_order,
    'insight_bitbucket_cloud' AS data_source,
    toUnixTimestamp64Milli(now64()) AS _version,
    _airbyte_extracted_at
FROM {{ source('bronze_bitbucket_cloud', 'pull_request_commits') }}
{% if is_incremental() %}
WHERE _airbyte_extracted_at > (SELECT max(_airbyte_extracted_at) FROM {{ this }})
{% endif %}
