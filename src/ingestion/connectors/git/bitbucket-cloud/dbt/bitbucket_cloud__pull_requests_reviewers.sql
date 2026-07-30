-- depends_on: {{ ref('bitbucket_cloud__bronze_promoted') }}
{{ config(
    materialized='incremental',
    unique_key='unique_key',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    schema='staging',
    tags=['bitbucket-cloud', 'silver:class_git_pull_requests_reviewers']
) }}

SELECT
    pr.tenant_id,
    pr.source_id,
    concat(COALESCE(pr.tenant_id, ''), ':', COALESCE(pr.source_id, ''), ':', COALESCE(pr.workspace, ''), ':', COALESCE(pr.repo_slug, ''), ':', toString(pr.id), ':', COALESCE(JSONExtractString(p, 'uuid'), '')) AS unique_key,
    COALESCE(pr.workspace, '') AS project_key,
    COALESCE(pr.repo_slug, '') AS repo_slug,
    COALESCE(pr.id, 0) AS pr_id,
    COALESCE(JSONExtractString(p, 'display_name'), '') AS reviewer_name,
    COALESCE(JSONExtractString(p, 'uuid'), '') AS reviewer_uuid,
    COALESCE(JSONExtractString(p, 'state'), '') AS status,
    if(JSONExtractBool(p, 'approved'), 1, 0) AS approved,
    -- participated_on is when this reviewer last approved / requested changes;
    -- absent for reviewers who never acted (parses to NULL).
    parseDateTimeBestEffortOrNull(JSONExtractString(p, 'participated_on')) AS reviewed_at,
    'insight_bitbucket_cloud' AS data_source,
    toUnixTimestamp64Milli(now64()) AS _version,
    pr._airbyte_extracted_at
FROM {{ source('bronze_bitbucket_cloud', 'pull_requests') }} AS pr FINAL
ARRAY JOIN JSONExtractArrayRaw(COALESCE(toString(pr.participants), '[]')) AS p
WHERE pr.record_type = 'item'
AND JSONExtractString(p, 'role') = 'REVIEWER'
{% if is_incremental() %}
AND pr._airbyte_extracted_at > (SELECT max(_airbyte_extracted_at) FROM {{ this }})
{% endif %}
