{% set _bronze_promoted = ref('bitbucket_cloud__bronze_promoted') %}
{{ config(
    materialized='incremental',
    unique_key='unique_key',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    schema='staging',
    tags=['bitbucket-cloud']
) }}

SELECT
    tenant_id,
    source_id,
    entity_key AS unique_key,
    COALESCE(workspace, '') AS project_key,
    COALESCE(repo_slug, '') AS repo_slug,
    repository_uuid,
    COALESCE(branch_name, '') AS branch_name,
    branch_head_sha,
    default_branch_name,
    commit_sha,
    parseDateTimeBestEffortOrNull(committed_at) AS committed_at,
    COALESCE(reachability_action, '') AS reachability_action,
    'insight_bitbucket_cloud' AS data_source,
    toUnixTimestamp64Milli(now64()) AS _version,
    _airbyte_extracted_at
FROM {{ source('bronze_bitbucket_cloud', 'commit_branch_reachability') }} FINAL
WHERE record_type = 'item'
{% if is_incremental() %}
AND _airbyte_extracted_at > (SELECT max(_airbyte_extracted_at) FROM {{ this }})
{% endif %}
