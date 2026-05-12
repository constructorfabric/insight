{{ config(
    materialized='incremental',
    unique_key='unique_key',
    incremental_strategy='append',
    engine='ReplacingMergeTree(_version)',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    schema='silver',
    tags=['silver']
) }}

SELECT
    fc.tenant_id,
    fc.source_id,
    fc.unique_key,
    fc.project_key,
    fc.repo_slug,
    fc.commit_hash,
    fc.file_path,
    fc.file_extension,
    fc.change_type,
    fc.lines_added,
    fc.lines_removed,
    multiIf(
        match(fc.file_path, '(?i)(\\.spec\\.|\\.test\\.|__tests__/|/tests?/)'), 'spec',
        match(fc.file_path, '(?i)(\\.lock$|package-lock\\.json|yarn\\.lock|poetry\\.lock|\\.ya?ml$|\\.toml$|\\.cfg$|\\.ini$)'), 'config',
        'code'
    ) AS file_category,
    c.author_name,
    c.author_email,
    c.person_key,
    c.date AS committed_at,
    c.week,
    c.is_merge_commit,
    fc.data_source,
    fc._version,
    fc._airbyte_extracted_at
FROM {{ ref('class_git_file_changes') }} AS fc
-- INNER JOIN: a file change without a matching commit cannot be attributed
-- to a person/week and has no usable downstream role. Enforcing correspondence
-- here avoids silent NULL-propagation through WHERE filters in metric models.
INNER JOIN {{ ref('fct_git_commit') }} AS c
    ON  c.tenant_id   = fc.tenant_id
    AND c.source_id   = fc.source_id
    AND c.project_key = fc.project_key
    AND c.repo_slug   = fc.repo_slug
    AND c.commit_hash = fc.commit_hash
{% if is_incremental() %}
WHERE fc._version > (SELECT max(_version) FROM {{ this }})
{% endif %}
