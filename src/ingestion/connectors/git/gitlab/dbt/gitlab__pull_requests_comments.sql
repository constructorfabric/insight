-- depends_on: {{ ref('gitlab__bronze_promoted') }}
{{ config(
    materialized='incremental',
    unique_key='unique_key',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    schema='staging',
    tags=['gitlab', 'silver:class_git_pull_requests_comments']
) }}

-- GitLab merge-request notes -> class_git_pull_requests_comments. System notes
-- (automated state-change entries, not human comments) are excluded so the
-- grain matches GitHub/Bitbucket review comments. pr_id is the merge request
-- iid. A note with a diff position is treated as inline.
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
    n.tenant_id,
    n.source_id,
    n.unique_key,
    COALESCE(p.project_key, '') AS project_key,
    COALESCE(p.repo_slug, '') AS repo_slug,
    COALESCE(n.mr_iid, 0) AS pr_id,
    COALESCE(n.id, 0) AS comment_id,
    COALESCE(n.body, '') AS content,
    COALESCE(n.author_username, '') AS author_name,
    toString(COALESCE(n.author_id, 0)) AS author_uuid,
    parseDateTimeBestEffortOrNull(n.created_at) AS created_at,
    parseDateTimeBestEffortOrNull(n.updated_at) AS updated_at,
    if(COALESCE(n.position_new_path, '') != '', 1, 0) AS is_inline,
    COALESCE(n.position_new_path, '') AS file_path,
    COALESCE(n.position_new_line, 0) AS line_number,
    'insight_gitlab' AS data_source,
    toUnixTimestamp64Milli(now64()) AS _version,
    n._airbyte_extracted_at
FROM {{ source('bronze_gitlab', 'merge_request_notes') }} AS n FINAL
LEFT JOIN proj AS p
    ON p.project_id = n.project_id
    AND p.tenant_id = n.tenant_id
    AND p.source_id = n.source_id
WHERE COALESCE(n.system, false) = false
{% if is_incremental() %}
AND n._airbyte_extracted_at > (SELECT max(_airbyte_extracted_at) FROM {{ this }})
{% endif %}
