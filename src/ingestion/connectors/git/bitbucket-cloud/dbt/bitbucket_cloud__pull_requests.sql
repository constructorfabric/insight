-- depends_on: {{ ref('bitbucket_cloud__bronze_promoted') }}
{{ config(
    materialized='incremental',
    unique_key='unique_key',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    schema='staging',
    tags=['bitbucket-cloud', 'silver:class_git_pull_requests']
) }}

WITH diffstat_generations AS (
    SELECT
        tenant_id,
        source_id,
        repository_uuid,
        pr_id,
        generation_id,
        pull_request_updated_on,
        pull_request_source_commit_hash,
        pull_request_destination_commit_hash,
        countIf(record_type = 'item') AS observed_count,
        maxIf(snapshot_item_count, record_type = 'snapshot_complete') AS expected_count,
        maxIf(_airbyte_extracted_at, record_type = 'snapshot_complete') AS completed_at,
        countIf(record_type = 'snapshot_complete' AND snapshot_available) AS completion_count
    FROM {{ source('bronze_bitbucket_cloud', 'pull_request_diffstat') }} FINAL
    GROUP BY tenant_id, source_id, repository_uuid, pr_id, generation_id,
        pull_request_updated_on, pull_request_source_commit_hash,
        pull_request_destination_commit_hash
    HAVING completion_count > 0 AND observed_count = expected_count
),
latest_diffstat_generation AS (
    SELECT
        tenant_id,
        source_id,
        repository_uuid,
        pr_id,
        argMax(generation_id, completed_at) AS generation_id,
        argMax(pull_request_updated_on, completed_at) AS pull_request_updated_on,
        argMax(pull_request_source_commit_hash, completed_at) AS pull_request_source_commit_hash,
        argMax(pull_request_destination_commit_hash, completed_at) AS pull_request_destination_commit_hash,
        max(completed_at) AS latest_completed_at
    FROM diffstat_generations
    GROUP BY tenant_id, source_id, repository_uuid, pr_id
),
diffstat AS (
    SELECT
        latest.tenant_id,
        latest.source_id,
        latest.repository_uuid,
        latest.pr_id,
        latest.pull_request_updated_on,
        latest.pull_request_source_commit_hash,
        latest.pull_request_destination_commit_hash,
        countIf(diff.record_type = 'item') AS files_changed,
        if(countIf(diff.record_type = 'item' AND (diff.lines_added IS NULL OR diff.lines_removed IS NULL)) > 0, NULL, sumIf(diff.lines_added, diff.record_type = 'item')) AS lines_added,
        if(countIf(diff.record_type = 'item' AND (diff.lines_added IS NULL OR diff.lines_removed IS NULL)) > 0, NULL, sumIf(diff.lines_removed, diff.record_type = 'item')) AS lines_removed,
        latest.latest_completed_at AS completed_at
    FROM latest_diffstat_generation AS latest
    LEFT JOIN {{ source('bronze_bitbucket_cloud', 'pull_request_diffstat') }} AS diff FINAL
        USING (tenant_id, source_id, repository_uuid, pr_id, generation_id)
    GROUP BY latest.tenant_id, latest.source_id, latest.repository_uuid, latest.pr_id,
        latest.pull_request_updated_on, latest.pull_request_source_commit_hash,
        latest.pull_request_destination_commit_hash, latest.latest_completed_at
),
activity_generations AS (
    SELECT
        tenant_id,
        source_id,
        repository_uuid,
        pr_id,
        generation_id,
        pull_request_updated_on,
        countIf(record_type = 'item') AS observed_count,
        maxIf(snapshot_item_count, record_type = 'snapshot_complete') AS expected_count,
        maxIf(_airbyte_extracted_at, record_type = 'snapshot_complete') AS completed_at,
        countIf(record_type = 'snapshot_complete' AND snapshot_available) AS completion_count
    FROM {{ source('bronze_bitbucket_cloud', 'pull_request_activity') }} FINAL
    GROUP BY tenant_id, source_id, repository_uuid, pr_id, generation_id, pull_request_updated_on
    HAVING completion_count > 0 AND observed_count = expected_count
),
latest_activity_generation AS (
    SELECT
        tenant_id,
        source_id,
        repository_uuid,
        pr_id,
        argMax(generation_id, completed_at) AS generation_id,
        argMax(pull_request_updated_on, completed_at) AS pull_request_updated_on,
        max(completed_at) AS latest_completed_at
    FROM activity_generations
    GROUP BY tenant_id, source_id, repository_uuid, pr_id
),
activity AS (
    SELECT
        latest.tenant_id,
        latest.source_id,
        latest.repository_uuid,
        latest.pr_id,
        latest.pull_request_updated_on,
        maxIf(event.activity_date, event.record_type = 'item' AND event.update_state IN ('MERGED', 'DECLINED', 'SUPERSEDED')) AS terminal_activity_date,
        latest.latest_completed_at AS completed_at
    FROM latest_activity_generation AS latest
    LEFT JOIN {{ source('bronze_bitbucket_cloud', 'pull_request_activity') }} AS event FINAL
        USING (tenant_id, source_id, repository_uuid, pr_id, generation_id)
    GROUP BY latest.tenant_id, latest.source_id, latest.repository_uuid, latest.pr_id,
        latest.pull_request_updated_on, latest.latest_completed_at
)
SELECT
    pr.tenant_id AS tenant_id,
    pr.source_id AS source_id,
    pr.entity_key AS unique_key,
    COALESCE(pr.workspace, '') AS project_key,
    COALESCE(pr.repo_slug, '') AS repo_slug,
    COALESCE(pr.id, 0) AS pr_id,
    COALESCE(pr.id, 0) AS pr_number,
    COALESCE(pr.title, '') AS title,
    COALESCE(pr.description, '') AS description,
    multiIf(
        pr.state = 'SUPERSEDED', 'DECLINED',
        COALESCE(pr.state, '')
    ) AS state,
    COALESCE(pr.author_display_name, '') AS author_name,
    '' AS author_email,
    COALESCE(pr.source_branch, '') AS source_branch,
    COALESCE(pr.destination_branch, '') AS destination_branch,
    parseDateTimeBestEffortOrNull(pr.created_on) AS created_on,
    parseDateTimeBestEffortOrNull(pr.updated_on) AS updated_on,
    parseDateTimeBestEffortOrNull(
        if(pr.state IN ('MERGED', 'DECLINED', 'SUPERSEDED'), COALESCE(activity.terminal_activity_date, ''), '')
    ) AS closed_on,
    COALESCE(pr.merge_commit_hash, '') AS merge_commit_hash,
    CAST(diffstat.files_changed, 'Nullable(Int64)') AS files_changed,
    CAST(diffstat.lines_added, 'Nullable(Int64)') AS lines_added,
    CAST(diffstat.lines_removed, 'Nullable(Int64)') AS lines_removed,
    'insight_bitbucket_cloud' AS data_source,
    toUnixTimestamp64Milli(now64()) AS _version,
    greatest(
        pr._airbyte_extracted_at,
        COALESCE(diffstat.completed_at, pr._airbyte_extracted_at),
        COALESCE(activity.completed_at, pr._airbyte_extracted_at)
    ) AS _airbyte_extracted_at
FROM {{ source('bronze_bitbucket_cloud', 'pull_requests') }} AS pr FINAL
LEFT JOIN diffstat
    ON diffstat.pr_id = pr.id
    AND diffstat.tenant_id = pr.tenant_id
    AND diffstat.source_id = pr.source_id
    AND diffstat.repository_uuid = pr.repository_uuid
    AND COALESCE(diffstat.pull_request_source_commit_hash, '') = COALESCE(pr.source_commit_hash, '')
    AND COALESCE(diffstat.pull_request_destination_commit_hash, '') = COALESCE(pr.destination_commit_hash, '')
LEFT JOIN activity
    ON activity.pr_id = pr.id
    AND activity.tenant_id = pr.tenant_id
    AND activity.source_id = pr.source_id
    AND activity.repository_uuid = pr.repository_uuid
WHERE pr.record_type = 'item'
{% if is_incremental() %}
AND greatest(
    pr._airbyte_extracted_at,
    COALESCE(diffstat.completed_at, pr._airbyte_extracted_at),
    COALESCE(activity.completed_at, pr._airbyte_extracted_at)
) > (SELECT max(_airbyte_extracted_at) FROM {{ this }})
{% endif %}
