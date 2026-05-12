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
    r.tenant_id,
    r.source_id,
    r.unique_key,
    r.project_key,
    r.repo_slug,
    r.pr_id,
    r.reviewer_name,
    r.reviewer_uuid,
    lower(r.reviewer_name) AS person_key,
    r.status,
    r.approved,
    r.reviewed_at,
    r.data_source,
    r._version,
    r._airbyte_extracted_at
FROM {{ ref('class_git_pull_requests_reviewers') }} AS r
{% if is_incremental() %}
WHERE r._version > (SELECT max(_version) FROM {{ this }})
{% endif %}
