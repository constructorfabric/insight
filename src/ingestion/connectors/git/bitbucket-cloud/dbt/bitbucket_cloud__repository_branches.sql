-- depends_on: {{ ref('bitbucket_cloud__bronze_promoted') }}
{{ config(
    materialized='table',
    unique_key='unique_key',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    schema='staging',
    tags=['bitbucket-cloud', 'silver:class_git_repository_branches']
) }}

WITH generations AS (
    SELECT
        tenant_id,
        source_id,
        bucket_id,
        generation_id,
        countIf(record_type = 'item') AS observed_count,
        maxIf(snapshot_item_count, record_type = 'snapshot_complete') AS expected_count,
        maxIf(_airbyte_extracted_at, record_type = 'snapshot_complete') AS completed_at,
        countIf(record_type = 'snapshot_complete' AND snapshot_available) AS completion_count
    FROM {{ source('bronze_bitbucket_cloud', 'branches') }} FINAL
    GROUP BY tenant_id, source_id, bucket_id, generation_id
    HAVING completion_count > 0 AND observed_count = expected_count
),
latest AS (
    SELECT
        tenant_id,
        source_id,
        bucket_id,
        argMax(generation_id, completed_at) AS generation_id
    FROM generations
    GROUP BY tenant_id, source_id, bucket_id
)
SELECT
    tenant_id,
    source_id,
    entity_key AS unique_key,
    COALESCE(workspace, '') AS project_key,
    COALESCE(repo_slug, '') AS repo_slug,
    COALESCE(name, '') AS branch_name,
    if(is_default, 1, 0) AS is_default,
    COALESCE(target_hash, '') AS last_commit_hash,
    parseDateTimeBestEffortOrNull(target_date) AS last_commit_date,
    'insight_bitbucket_cloud' AS data_source,
    toUnixTimestamp64Milli(now64()) AS _version,
    _airbyte_extracted_at
FROM {{ source('bronze_bitbucket_cloud', 'branches') }} AS branch FINAL
INNER JOIN latest USING (tenant_id, source_id, bucket_id, generation_id)
WHERE record_type = 'item'
