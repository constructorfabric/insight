-- depends_on: {{ ref('gitlab__bronze_promoted') }}
{{ config(
    materialized='incremental',
    unique_key='unique_key',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    schema='staging',
    tags=['gitlab', 'silver:class_git_pull_requests_reviewers']
) }}

-- GitLab merge-request approvals -> class_git_pull_requests_reviewers, one row
-- per approver (array-joined out of approved_by). The approvals endpoint lists
-- only users who approved, so status is APPROVED / approved = 1; it carries no
-- per-approver timestamp (reviewed_at null). unique_key is synthesised per
-- (merge request, reviewer) since the bronze row is per merge request.
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
    a.tenant_id,
    a.source_id,
    concat(
        COALESCE(a.tenant_id, ''), ':',
        COALESCE(a.source_id, ''), ':',
        toString(a.project_id), ':',
        toString(COALESCE(a.mr_iid, 0)), ':',
        toString(JSONExtractInt(rv, 'id'))
    ) AS unique_key,
    COALESCE(p.project_key, '') AS project_key,
    COALESCE(p.repo_slug, '') AS repo_slug,
    COALESCE(a.mr_iid, 0) AS pr_id,
    COALESCE(JSONExtractString(rv, 'username'), '') AS reviewer_name,
    toString(JSONExtractInt(rv, 'id')) AS reviewer_uuid,
    'APPROVED' AS status,
    1 AS approved,
    CAST(NULL AS Nullable(DateTime)) AS reviewed_at,
    'insight_gitlab' AS data_source,
    toUnixTimestamp64Milli(now64()) AS _version,
    a._airbyte_extracted_at
FROM {{ source('bronze_gitlab', 'merge_request_approvals') }} AS a FINAL
ARRAY JOIN JSONExtractArrayRaw(COALESCE(toString(a.approved_by), '[]')) AS rv
LEFT JOIN proj AS p
    ON p.project_id = a.project_id
    AND p.tenant_id = a.tenant_id
    AND p.source_id = a.source_id
{% if is_incremental() %}
WHERE a._airbyte_extracted_at > (SELECT max(_airbyte_extracted_at) FROM {{ this }})
{% endif %}
