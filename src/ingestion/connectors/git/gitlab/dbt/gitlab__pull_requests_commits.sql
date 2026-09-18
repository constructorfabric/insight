-- depends_on: {{ ref('gitlab__bronze_promoted') }}
{{ config(
    materialized='incremental',
    unique_key='unique_key',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    schema='staging',
    tags=['gitlab', 'silver:class_git_pull_requests_commits']
) }}

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
    c.tenant_id AS tenant_id,
    c.source_id AS source_id,
    c.unique_key AS unique_key,
    p.project_key AS project_key,
    p.repo_slug AS repo_slug,
    COALESCE(c.mr_iid, 0) AS pr_id,
    COALESCE(c.sha, '') AS commit_hash,
    toInt64(0) AS commit_order,
    'insight_gitlab' AS data_source,
    toUnixTimestamp64Milli(now64()) AS _version,
    c._airbyte_extracted_at AS _airbyte_extracted_at
FROM {{ source('bronze_gitlab', 'pull_request_commits') }} AS c FINAL
INNER JOIN projects AS p
    ON p.tenant_id = c.tenant_id
    AND p.source_id = c.source_id
    AND p.project_id = c.project_id
{% if is_incremental() %}
WHERE c._airbyte_extracted_at > (SELECT max(_airbyte_extracted_at) FROM {{ this }})
{% endif %}
