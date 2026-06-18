-- depends_on: {{ ref('gitlab__bronze_promoted') }}
{{ config(
    materialized='incremental',
    unique_key='unique_key',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    schema='staging',
    tags=['gitlab', 'silver:class_git_pull_requests_commits']
) }}

-- GitLab merge-request commits -> class_git_pull_requests_commits (PR<->commit
-- link). pr_id is the merge request iid. commit_order is not provided by the
-- API (0, matching Bitbucket Cloud).
WITH proj AS (
    SELECT
        tenant_id,
        source_id,
        id AS project_id,
        COALESCE(namespace_full_path, '') AS project_key,
        COALESCE(path, '') AS repo_slug
    FROM {{ source('bronze_gitlab', 'projects') }} FINAL
)
SELECT
    mc.tenant_id,
    mc.source_id,
    mc.unique_key,
    COALESCE(p.project_key, '') AS project_key,
    COALESCE(p.repo_slug, '') AS repo_slug,
    COALESCE(mc.mr_iid, 0) AS pr_id,
    COALESCE(mc.id, '') AS commit_hash,
    0 AS commit_order,
    'insight_gitlab' AS data_source,
    toUnixTimestamp64Milli(now64()) AS _version,
    mc._airbyte_extracted_at
FROM {{ source('bronze_gitlab', 'merge_request_commits') }} AS mc
LEFT JOIN proj AS p
    ON p.project_id = mc.project_id
    AND p.tenant_id = mc.tenant_id
    AND p.source_id = mc.source_id
{% if is_incremental() %}
WHERE mc._airbyte_extracted_at > (SELECT max(_airbyte_extracted_at) FROM {{ this }})
{% endif %}
