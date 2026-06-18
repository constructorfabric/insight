-- depends_on: {{ ref('gitlab__bronze_promoted') }}
{{ config(
    materialized='incremental',
    unique_key='unique_key',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    schema='staging',
    tags=['gitlab', 'silver:class_git_file_changes']
) }}

-- project_key / repo_slug resolved from projects. change_type derived from the
-- new_file / deleted_file / renamed_file flags. source_type is not provided by
-- GitLab diffs.
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
    fc.tenant_id,
    fc.source_id,
    fc.unique_key,
    COALESCE(p.project_key, '') AS project_key,
    COALESCE(p.repo_slug, '') AS repo_slug,
    COALESCE(fc.commit_sha, '') AS commit_hash,
    COALESCE(fc.new_path, fc.old_path, '') AS file_path,
    if(
        length(splitByChar('.', COALESCE(fc.new_path, fc.old_path, ''))) > 1,
        arrayElement(splitByChar('.', COALESCE(fc.new_path, fc.old_path, '')), -1),
        ''
    ) AS file_extension,
    multiIf(
        fc.new_file = true, 'added',
        fc.deleted_file = true, 'deleted',
        fc.renamed_file = true, 'renamed',
        'modified'
    ) AS change_type,
    COALESCE(fc.lines_added, 0) AS lines_added,
    COALESCE(fc.lines_removed, 0) AS lines_removed,
    '' AS source_type,
    'insight_gitlab' AS data_source,
    toUnixTimestamp64Milli(now64()) AS _version,
    fc._airbyte_extracted_at
FROM {{ source('bronze_gitlab', 'commit_file_changes') }} AS fc
LEFT JOIN proj AS p
    ON p.project_id = fc.project_id
    AND p.tenant_id = fc.tenant_id
    AND p.source_id = fc.source_id
{% if is_incremental() %}
WHERE fc._airbyte_extracted_at > (SELECT max(_airbyte_extracted_at) FROM {{ this }})
{% endif %}
