{{ config(
    materialized='incremental',
    unique_key='unique_key',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    schema='staging',
    tags=['github', 'silver:class_git_pull_requests']
) }}

SELECT
    tenant_id,
    source_id,
    unique_key,
    COALESCE(repo_owner, '') AS project_key,
    COALESCE(repo_name, '') AS repo_slug,
    COALESCE(database_id, 0) AS pr_id,
    COALESCE(number, 0) AS pr_number,
    COALESCE(title, '') AS title,
    COALESCE(body, '') AS description,
    COALESCE(state, '') AS state,
    COALESCE(author_login, '') AS author_name,
    COALESCE(author_email, '') AS author_email,
    COALESCE(head_ref, '') AS source_branch,
    COALESCE(base_ref, '') AS destination_branch,
    parseDateTimeBestEffortOrNull(created_at) AS created_on,
    parseDateTimeBestEffortOrNull(updated_at) AS updated_on,
    parseDateTimeBestEffortOrNull(closed_at) AS closed_on,
    COALESCE(merge_commit_sha, '') AS merge_commit_hash,
    COALESCE(changed_files, 0) AS files_changed,
    COALESCE(additions, 0) AS lines_added,
    COALESCE(deletions, 0) AS lines_removed,
    'insight_github' AS data_source,
    toUnixTimestamp64Milli(now64()) AS _version,
    _airbyte_extracted_at
FROM {{ source('bronze_github', 'pull_requests') }}
{% if is_incremental() %}
WHERE _airbyte_extracted_at > (SELECT max(_airbyte_extracted_at) FROM {{ this }})
{% endif %}
