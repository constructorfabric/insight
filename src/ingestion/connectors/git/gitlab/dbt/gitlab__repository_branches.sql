-- depends_on: {{ ref('gitlab__bronze_promoted') }}
{{ config(
    materialized='incremental',
    unique_key='unique_key',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    schema='staging',
    tags=['gitlab', 'silver:class_git_repository_branches']
) }}

-- project_key / repo_slug resolved from projects (branch bronze carries only
-- numeric project_id). last_commit_date is null: the branches stream keeps the
-- head sha but not its commit date.
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
    b.tenant_id,
    b.source_id,
    b.unique_key,
    COALESCE(p.project_key, '') AS project_key,
    COALESCE(p.repo_slug, '') AS repo_slug,
    COALESCE(b.name, '') AS branch_name,
    if(b.`default` = true, 1, 0) AS is_default,
    COALESCE(b.commit_sha, '') AS last_commit_hash,
    CAST(NULL AS Nullable(DateTime)) AS last_commit_date,
    'insight_gitlab' AS data_source,
    toUnixTimestamp64Milli(now64()) AS _version,
    b._airbyte_extracted_at
FROM {{ source('bronze_gitlab', 'branches') }} AS b FINAL
LEFT JOIN proj AS p
    ON p.project_id = b.project_id
    AND p.tenant_id = b.tenant_id
    AND p.source_id = b.source_id
{% if is_incremental() %}
WHERE b._airbyte_extracted_at > (SELECT max(_airbyte_extracted_at) FROM {{ this }})
{% endif %}
