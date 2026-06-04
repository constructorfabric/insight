{{ config(
    materialized='incremental',
    unique_key='unique_key',
    incremental_strategy='delete+insert',
    engine='ReplacingMergeTree(_version)',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    schema='silver',
    tags=['silver']
) }}

WITH author_email_xwalk AS (
    SELECT
        tenant_id,
        name_key,
        argMax(email, (n, email)) AS email
    FROM (
        SELECT
            tenant_id,
            lower(author_name)  AS name_key,
            lower(author_email) AS email,
            count()             AS n
        FROM {{ ref('fct_git_commit') }} FINAL
        WHERE author_email != '' AND author_name != ''
        GROUP BY tenant_id, name_key, email
    )
    GROUP BY tenant_id, name_key
)
SELECT
    pr.tenant_id,
    pr.source_id,
    pr.unique_key,
    pr.project_key,
    pr.repo_slug,
    pr.pr_id,
    pr.pr_number,
    pr.title,
    pr.state,
    lower(pr.state) AS state_norm,
    pr.author_name,
    pr.author_email,
    if(
        pr.author_email != '',
        lower(pr.author_email),
        coalesce(nullif(xw.email, ''), lower(pr.author_name))
    ) AS person_key,
    pr.source_branch,
    pr.destination_branch,
    pr.created_on,
    pr.updated_on,
    pr.closed_on,
    pr.merge_commit_hash,
    pr.files_changed,
    pr.lines_added,
    pr.lines_removed,
    -- Clamp negative diffs (dirty data where closed_on < created_on) to NULL
    -- so avg_pr_cycle_time_h and percentiles aren't skewed by impossible values.
    if(
        lower(pr.state) = 'merged'
        AND pr.closed_on IS NOT NULL
        AND pr.created_on IS NOT NULL
        AND pr.closed_on >= pr.created_on,
        dateDiff('second', pr.created_on, pr.closed_on) / 3600.0,
        NULL
    ) AS cycle_time_h,
    pr.data_source,
    pr._version,
    pr._airbyte_extracted_at
FROM {{ ref('class_git_pull_requests') }} AS pr FINAL
LEFT JOIN author_email_xwalk AS xw
    ON xw.tenant_id = pr.tenant_id
   AND xw.name_key = lower(pr.author_name)
{% if is_incremental() %}
WHERE pr._version > (SELECT max(_version) FROM {{ this }})
{% endif %}
