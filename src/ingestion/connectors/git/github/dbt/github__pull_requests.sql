-- depends_on: {{ ref('github__bronze_promoted') }}
{{ config(
    materialized='incremental',
    unique_key='unique_key',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    schema='staging',
    tags=['github', 'silver:class_git_pull_requests']
) }}

-- Diff totals and the author email come from the GraphQL list node, which is
-- one request per page of pull requests; the REST list carries neither.
WITH diff_stats AS (
    SELECT
        tenant_id,
        source_id,
        repo_full_name,
        pull_number,
        additions,
        deletions,
        changed_files,
        author_email,
        _airbyte_extracted_at,
        1 AS matched
    FROM {{ source('bronze_github', 'pull_request_diff_stats') }} FINAL
)
SELECT
    pr.tenant_id AS tenant_id,
    pr.source_id AS source_id,
    pr.unique_key AS unique_key,
    splitByChar('/', COALESCE(pr.repo_full_name, ''))[1] AS project_key,
    splitByChar('/', COALESCE(pr.repo_full_name, ''))[2] AS repo_slug,
    COALESCE(pr.number, 0) AS pr_id,
    COALESCE(pr.number, 0) AS pr_number,
    COALESCE(pr.title, '') AS title,
    COALESCE(pr.body, '') AS description,
    -- REST reports a merged pull request as closed, so merged_at decides first.
    multiIf(
        COALESCE(pr.merged_at, '') != '', 'MERGED',
        pr.state = 'open', 'OPEN',
        pr.state = 'closed', 'CLOSED',
        upper(COALESCE(pr.state, ''))
    ) AS state,
    COALESCE(pr.author_login, '') AS author_name,
    COALESCE(ds.author_email, '') AS author_email,
    COALESCE(pr.head_ref, '') AS source_branch,
    COALESCE(pr.base_ref, '') AS destination_branch,
    parseDateTimeBestEffortOrNull(pr.created_at) AS created_on,
    parseDateTimeBestEffortOrNull(pr.updated_at) AS updated_on,
    parseDateTimeBestEffortOrNull(COALESCE(pr.closed_at, pr.merged_at)) AS closed_on,
    COALESCE(pr.merge_commit_sha, '') AS merge_commit_hash,
    -- An unmatched join partner and a genuinely empty pull request both read
    -- as 0 through COALESCE; only the marker separates "not collected yet"
    -- from "changed nothing", and the class columns are nullable to say so.
    if(ds.matched = 1, toNullable(toInt64(COALESCE(ds.changed_files, 0))), NULL) AS files_changed,
    if(ds.matched = 1, toNullable(toInt64(COALESCE(ds.additions, 0))), NULL) AS lines_added,
    if(ds.matched = 1, toNullable(toInt64(COALESCE(ds.deletions, 0))), NULL) AS lines_removed,
    'insight_github' AS data_source,
    toUnixTimestamp64Milli(now64()) AS _version,
    -- Diff stats are their own stream: a late arrival must re-trigger the pull
    -- request row, which is not re-fetched on its own.
    greatest(
        pr._airbyte_extracted_at,
        COALESCE(ds._airbyte_extracted_at, pr._airbyte_extracted_at)
    ) AS _airbyte_extracted_at
FROM {{ source('bronze_github', 'pull_requests') }} AS pr FINAL
LEFT JOIN diff_stats AS ds
    ON ds.tenant_id = pr.tenant_id
    AND ds.source_id = pr.source_id
    AND ds.repo_full_name = pr.repo_full_name
    AND ds.pull_number = pr.number
{% if is_incremental() %}
WHERE greatest(
    pr._airbyte_extracted_at,
    COALESCE(ds._airbyte_extracted_at, pr._airbyte_extracted_at)
) > (SELECT max(_airbyte_extracted_at) FROM {{ this }})
{% endif %}
