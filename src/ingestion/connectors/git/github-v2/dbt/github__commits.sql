{{ config(
    materialized='incremental',
    unique_key='unique_key',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    schema='staging',
    tags=['github', 'silver:class_git_commits']
) }}

SELECT
    tenant_id,
    source_id,
    unique_key,
    COALESCE(repo_owner, '') AS project_key,
    COALESCE(repo_name, '') AS repo_slug,
    COALESCE(sha, '') AS commit_hash,
    COALESCE(branch_name, '') AS branch,
    COALESCE(author_name, '') AS author_name,
    COALESCE(author_email, '') AS author_email,
    COALESCE(committer_name, '') AS committer_name,
    COALESCE(committer_email, '') AS committer_email,
    COALESCE(message, '') AS message,
    parseDateTimeBestEffortOrNull(committed_date) AS date,
    COALESCE(changed_files, 0) AS files_changed,
    COALESCE(additions, 0) AS lines_added,
    COALESCE(deletions, 0) AS lines_removed,
    -- parent_hashes arrives as a JSON-array string (Airbyte serializes the
    -- connector's array field into the Nullable(String) bronze column), so
    -- count elements with JSONLength — plain length() would count characters
    -- and flag every commit with a parent (e.g. `["sha"]`) as a merge.
    if(JSONLength(COALESCE(parent_hashes, '')) > 1, 1, 0) AS is_merge_commit,
    'insight_github' AS data_source,
    toUnixTimestamp64Milli(now64()) AS _version,
    _airbyte_extracted_at
FROM {{ source('bronze_github', 'commits') }}
{% if is_incremental() %}
WHERE _airbyte_extracted_at > (SELECT max(_airbyte_extracted_at) FROM {{ this }})
{% endif %}
