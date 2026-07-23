-- depends_on: {{ ref('bitbucket_cloud__bronze_promoted') }}
{{ config(
    materialized='incremental',
    unique_key='unique_key',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    schema='staging',
    tags=['bitbucket-cloud', 'silver:class_git_commits']
) }}

WITH file_change_generations AS (
    SELECT
        tenant_id,
        source_id,
        repository_uuid,
        sha,
        generation_id,
        countIf(record_type = 'item') AS observed_count,
        maxIf(snapshot_item_count, record_type = 'snapshot_complete') AS expected_count,
        maxIf(_airbyte_extracted_at, record_type = 'snapshot_complete') AS completed_at,
        countIf(record_type = 'snapshot_complete' AND snapshot_available) AS completion_count
    FROM {{ source('bronze_bitbucket_cloud', 'file_changes') }} FINAL
    GROUP BY tenant_id, source_id, repository_uuid, sha, generation_id
    HAVING completion_count > 0 AND observed_count = expected_count
),
latest_file_change_generation AS (
    SELECT
        tenant_id,
        source_id,
        repository_uuid,
        sha,
        argMax(generation_id, completed_at) AS generation_id,
        max(completed_at) AS completed_at
    FROM file_change_generations
    GROUP BY tenant_id, source_id, repository_uuid, sha
),
file_changes AS (
    SELECT
        change.tenant_id,
        change.source_id,
        change.repository_uuid,
        change.sha,
        count() AS files_changed,
        if(countIf(change.additions IS NULL OR change.deletions IS NULL) > 0, NULL, sum(change.additions)) AS lines_added,
        if(countIf(change.additions IS NULL OR change.deletions IS NULL) > 0, NULL, sum(change.deletions)) AS lines_removed,
        max(latest.completed_at) AS completed_at
    FROM {{ source('bronze_bitbucket_cloud', 'file_changes') }} AS change FINAL
    INNER JOIN latest_file_change_generation AS latest
        USING (tenant_id, source_id, repository_uuid, sha, generation_id)
    WHERE change.record_type = 'item'
    GROUP BY change.tenant_id, change.source_id, change.repository_uuid, change.sha
),
empty_file_changes AS (
    SELECT
        latest.tenant_id,
        latest.source_id,
        latest.repository_uuid,
        latest.sha,
        0 AS files_changed,
        CAST(0, 'Nullable(Int64)') AS lines_added,
        CAST(0, 'Nullable(Int64)') AS lines_removed,
        latest.completed_at
    FROM latest_file_change_generation AS latest
    LEFT ANTI JOIN file_changes AS change
        USING (tenant_id, source_id, repository_uuid, sha)
),
complete_file_changes AS (
    SELECT * FROM file_changes
    UNION ALL
    SELECT * FROM empty_file_changes
)
SELECT
    c.tenant_id,
    c.source_id,
    c.entity_key AS unique_key,
    COALESCE(c.workspace, '') AS project_key,
    COALESCE(c.repo_slug, '') AS repo_slug,
    COALESCE(c.hash, '') AS commit_hash,
    COALESCE(c.branch_name, '') AS branch,
    COALESCE(c.author_name, '') AS author_name,
    COALESCE(c.author_email, '') AS author_email,
    COALESCE(c.committer_name, '') AS committer_name,
    COALESCE(c.committer_email, '') AS committer_email,
    COALESCE(c.message, '') AS message,
    parseDateTimeBestEffortOrNull(c.date) AS date,
    CAST(fc.files_changed, 'Nullable(Int64)') AS files_changed,
    CAST(fc.lines_added, 'Nullable(Int64)') AS lines_added,
    CAST(fc.lines_removed, 'Nullable(Int64)') AS lines_removed,
    if(JSONLength(COALESCE(toString(c.parent_hashes), '[]')) > 1, 1, 0) AS is_merge_commit,
    'insight_bitbucket_cloud' AS data_source,
    toUnixTimestamp64Milli(now64()) AS _version,
    greatest(c._airbyte_extracted_at, COALESCE(fc.completed_at, c._airbyte_extracted_at)) AS _airbyte_extracted_at
FROM {{ source('bronze_bitbucket_cloud', 'commits') }} AS c FINAL
LEFT JOIN complete_file_changes AS fc ON fc.sha = c.hash
    AND fc.tenant_id = c.tenant_id
    AND fc.source_id = c.source_id
    AND fc.repository_uuid = c.repository_uuid
WHERE c.record_type = 'item'
{% if is_incremental() %}
AND greatest(c._airbyte_extracted_at, COALESCE(fc.completed_at, c._airbyte_extracted_at))
    > (SELECT max(_airbyte_extracted_at) FROM {{ this }})
{% endif %}
