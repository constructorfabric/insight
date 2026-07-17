-- depends_on: {{ ref('bitbucket_cloud__bronze_promoted') }}
{{ config(
    materialized='incremental',
    unique_key='unique_key',
    order_by=['unique_key'],
    on_schema_change='append_new_columns',
    settings={'allow_nullable_key': 1},
    schema='staging',
    tags=['bitbucket-cloud', 'silver:class_git_commits']
) }}

SELECT
    c.tenant_id,
    c.source_id,
    c.unique_key,
    COALESCE(c.workspace, '') AS project_key,
    COALESCE(c.repo_slug, '') AS repo_slug,
    COALESCE(c.hash, '') AS commit_hash,
    COALESCE(c.branch_name, '') AS branch,
    COALESCE(c.author_name, '') AS author_name,
    COALESCE(c.author_email, '') AS author_email,
    '' AS committer_name,
    '' AS committer_email,
    COALESCE(c.message, '') AS message,
    parseDateTimeBestEffortOrNull(c.date) AS date,
    toInt64(COALESCE(fc.files_changed, 0)) AS files_changed,
    toInt64(COALESCE(fc.lines_added, 0)) AS lines_added,
    toInt64(COALESCE(fc.lines_removed, 0)) AS lines_removed,
    if(JSONLength(COALESCE(toString(c.parent_hashes), '[]')) > 1, 1, 0) AS is_merge_commit,
    'insight_bitbucket_cloud' AS data_source,
    toUnixTimestamp64Milli(now64()) AS _version,
    c._airbyte_extracted_at
FROM {{ source('bronze_bitbucket_cloud', 'commits') }} AS c FINAL
LEFT JOIN (
    SELECT
        tenant_id,
        workspace,
        repo_slug,
        sha,
        count() AS files_changed,
        SUM(COALESCE(additions, 0)) AS lines_added,
        SUM(COALESCE(deletions, 0)) AS lines_removed
    -- FINAL: dedup file_changes before count()/SUM, else bronze dupes inflate
    -- files_changed / lines_added / lines_removed (baked into one row, RMT
    -- downstream cannot undo it). See ADR-0001.
    FROM {{ source('bronze_bitbucket_cloud', 'file_changes') }} FINAL
    GROUP BY tenant_id, workspace, repo_slug, sha
) AS fc ON fc.sha = c.hash
    AND fc.tenant_id = c.tenant_id
    AND fc.workspace = c.workspace
    AND fc.repo_slug = c.repo_slug
{% if is_incremental() %}
WHERE c._airbyte_extracted_at > (SELECT max(_airbyte_extracted_at) FROM {{ this }})
{% endif %}
