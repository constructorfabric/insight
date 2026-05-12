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
    c.tenant_id,
    c.source_id,
    c.unique_key,
    c.project_key,
    c.repo_slug,
    c.commit_hash,
    c.branch,
    c.author_name,
    c.author_email,
    if(c.author_email != '', lower(c.author_email), lower(c.author_name)) AS person_key,
    c.committer_name,
    c.committer_email,
    c.message,
    c.date,
    toStartOfWeek(c.date, 1) AS week,
    c.files_changed,
    c.lines_added,
    c.lines_removed,
    c.is_merge_commit,
    c.data_source,
    c._version,
    c._airbyte_extracted_at
FROM {{ ref('class_git_commits') }} AS c
{% if is_incremental() %}
WHERE c._version > (SELECT max(_version) FROM {{ this }})
{% endif %}
