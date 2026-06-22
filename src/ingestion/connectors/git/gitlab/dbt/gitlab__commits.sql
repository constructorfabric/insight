-- depends_on: {{ ref('gitlab__bronze_promoted') }}
{{ config(
    materialized='incremental',
    unique_key='unique_key',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    schema='staging',
    tags=['gitlab', 'silver:class_git_commits']
) }}

-- lines_added / lines_removed come from the commit's own stats (present for
-- every commit). files_changed is the per-commit count from commit_file_changes,
-- which the connector only collects for default-branch non-merge commits — so
-- it is 0 for commits outside that set. branch is not stored on the commit row.
WITH proj AS (
    SELECT
        tenant_id,
        source_id,
        id AS project_id,
        COALESCE(namespace_full_path, '') AS project_key,
        COALESCE(path, '') AS repo_slug
    FROM {{ source('bronze_gitlab', 'projects') }} FINAL
),
fc AS (
    SELECT
        tenant_id,
        source_id,
        project_id,
        commit_sha,
        count() AS files_changed
    -- FINAL: dedup file_changes before count() so bronze dupes don't inflate
    -- files_changed (baked into one row). See ADR-0001.
    FROM {{ source('bronze_gitlab', 'commit_file_changes') }} FINAL
    GROUP BY tenant_id, source_id, project_id, commit_sha
)
SELECT
    c.tenant_id AS tenant_id,
    c.source_id AS source_id,
    c.unique_key AS unique_key,
    COALESCE(p.project_key, '') AS project_key,
    COALESCE(p.repo_slug, '') AS repo_slug,
    COALESCE(c.id, '') AS commit_hash,
    '' AS branch,
    COALESCE(c.author_name, '') AS author_name,
    COALESCE(c.author_email, '') AS author_email,
    COALESCE(c.committer_name, '') AS committer_name,
    COALESCE(c.committer_email, '') AS committer_email,
    COALESCE(c.message, '') AS message,
    parseDateTimeBestEffortOrNull(c.committed_date) AS date,
    COALESCE(f.files_changed, 0) AS files_changed,
    COALESCE(c.stats_additions, 0) AS lines_added,
    COALESCE(c.stats_deletions, 0) AS lines_removed,
    if(COALESCE(c.parent_count, 0) > 1, 1, 0) AS is_merge_commit,
    'insight_gitlab' AS data_source,
    toUnixTimestamp64Milli(now64()) AS _version,
    c._airbyte_extracted_at
FROM {{ source('bronze_gitlab', 'commits') }} AS c
LEFT JOIN proj AS p
    ON p.project_id = c.project_id
    AND p.tenant_id = c.tenant_id
    AND p.source_id = c.source_id
LEFT JOIN fc AS f
    ON f.project_id = c.project_id
    AND f.commit_sha = c.id
    AND f.tenant_id = c.tenant_id
    AND f.source_id = c.source_id
{% if is_incremental() %}
WHERE c._airbyte_extracted_at > (SELECT max(_airbyte_extracted_at) FROM {{ this }})
{% endif %}
