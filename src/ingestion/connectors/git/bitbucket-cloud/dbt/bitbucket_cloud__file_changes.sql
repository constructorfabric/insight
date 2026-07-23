-- depends_on: {{ ref('bitbucket_cloud__bronze_promoted') }}
{{ config(
    materialized='incremental',
    unique_key='unique_key',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    schema='staging',
    tags=['bitbucket-cloud', 'silver:class_git_file_changes']
) }}

WITH generations AS (
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
latest AS (
    SELECT
        tenant_id,
        source_id,
        repository_uuid,
        sha,
        argMax(generation_id, completed_at) AS generation_id,
        max(completed_at) AS latest_completed_at
    FROM generations
    GROUP BY tenant_id, source_id, repository_uuid, sha
)
SELECT
    change.tenant_id,
    change.source_id,
    change.entity_key AS unique_key,
    COALESCE(change.workspace, '') AS project_key,
    COALESCE(change.repo_slug, '') AS repo_slug,
    COALESCE(change.sha, '') AS commit_hash,
    COALESCE(change.filename, '') AS file_path,
    -- File extension: last segment after the final '.', empty when none.
    -- Earlier shape (issue #494) used `position('.', filename) > 0` as the
    -- guard — but ClickHouse `position` is function-style
    -- `position(haystack, needle)`, so this asked "is the string `filename`
    -- present inside the single character '.'?" — always false. Result:
    -- `file_extension` was empty for 100% of rows. Length check on the
    -- split array is more robust than a fixed `position(filename, '.') > 0`
    -- swap because it correctly returns '' for extensionless paths like
    -- `Makefile` (where the position-based guard would also fire 0 by
    -- accident, but the array-length guard is the explicit predicate).
    if(
        length(splitByChar('.', COALESCE(change.filename, ''))) > 1,
        arrayElement(splitByChar('.', COALESCE(change.filename, '')), -1),
        ''
    ) AS file_extension,
    COALESCE(change.status, '') AS change_type,
    change.additions AS lines_added,
    change.deletions AS lines_removed,
    COALESCE(change.source_type, '') AS source_type,
    'insight_bitbucket_cloud' AS data_source,
    toUnixTimestamp64Milli(now64()) AS _version,
    latest.latest_completed_at AS _airbyte_extracted_at
FROM {{ source('bronze_bitbucket_cloud', 'file_changes') }} AS change FINAL
INNER JOIN latest USING (tenant_id, source_id, repository_uuid, sha, generation_id)
WHERE change.record_type = 'item'
{% if is_incremental() %}
AND latest.latest_completed_at > (SELECT max(_airbyte_extracted_at) FROM {{ this }})
{% endif %}
