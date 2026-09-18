-- depends_on: {{ ref('gitlab__bronze_promoted') }}
{{ config(
    materialized='incremental',
    unique_key='unique_key',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    schema='staging',
    tags=['gitlab', 'silver:class_git_pull_requests_comments']
) }}

-- User-written notes only: system notes are GitLab's own log of the merge
-- request (approvals, pushes, label changes) and feed the event classes.
-- A DiffNote carries a position, which is what makes a comment inline.
WITH projects AS (
    SELECT
        tenant_id,
        source_id,
        id AS project_id,
        COALESCE(namespace_full_path, '') AS project_key,
        COALESCE(path, '') AS repo_slug
    FROM {{ source('bronze_gitlab', 'repositories') }} FINAL
)

SELECT
    n.tenant_id AS tenant_id,
    n.source_id AS source_id,
    n.unique_key AS unique_key,
    p.project_key AS project_key,
    p.repo_slug AS repo_slug,
    COALESCE(n.mr_iid, 0) AS pr_id,
    COALESCE(n.id, 0) AS comment_id,
    COALESCE(n.body, '') AS content,
    COALESCE(n.author_username, '') AS author_name,
    toString(COALESCE(n.author_id, 0)) AS author_uuid,
    parseDateTimeBestEffortOrNull(n.created_at) AS created_at,
    parseDateTimeBestEffortOrNull(n.updated_at) AS updated_at,
    if(COALESCE(n.position_new_path, '') != '' OR COALESCE(n.position_old_path, '') != '', 1, 0) AS is_inline,
    COALESCE(NULLIF(n.position_new_path, ''), n.position_old_path, '') AS file_path,
    COALESCE(n.position_new_line, n.position_old_line, 0) AS line_number,
    'insight_gitlab' AS data_source,
    toUnixTimestamp64Milli(now64()) AS _version,
    n._airbyte_extracted_at AS _airbyte_extracted_at
FROM {{ source('bronze_gitlab', 'pull_request_notes') }} AS n FINAL
INNER JOIN projects AS p
    ON p.tenant_id = n.tenant_id
    AND p.source_id = n.source_id
    AND p.project_id = n.project_id
WHERE NOT COALESCE(n.system, false)
{% if is_incremental() %}
  AND n._airbyte_extracted_at > (SELECT max(_airbyte_extracted_at) FROM {{ this }})
{% endif %}
