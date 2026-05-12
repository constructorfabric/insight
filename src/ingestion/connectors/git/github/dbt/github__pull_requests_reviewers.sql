{{ config(
    materialized='incremental',
    unique_key='unique_key',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    schema='staging',
    tags=['github', 'silver:class_git_pull_requests_reviewers']
) }}

SELECT
    tenant_id,
    source_id,
    unique_key,
    COALESCE(repo_owner, '') AS project_key,
    COALESCE(repo_name, '') AS repo_slug,
    COALESCE(pr_database_id, 0) AS pr_id,
    COALESCE(author_login, '') AS reviewer_name,
    toString(COALESCE(author_database_id, 0)) AS reviewer_uuid,
    COALESCE(state, '') AS status,
    if(state = 'APPROVED', 1, 0) AS approved,
    parseDateTimeBestEffortOrNull(submitted_at) AS reviewed_at,
    'insight_github' AS data_source,
    toUnixTimestamp64Milli(now64()) AS _version,
    _airbyte_extracted_at
FROM {{ source('bronze_github', 'pull_request_reviews') }}
{% if is_incremental() %}
WHERE _airbyte_extracted_at > (SELECT max(_airbyte_extracted_at) FROM {{ this }})
{% endif %}
