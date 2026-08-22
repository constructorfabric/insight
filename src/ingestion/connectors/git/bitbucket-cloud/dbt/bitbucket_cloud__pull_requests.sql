-- depends_on: {{ ref('bitbucket_cloud__bronze_promoted') }}
{{ config(
    materialized='incremental',
    unique_key='unique_key',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    schema='staging',
    tags=['bitbucket-cloud', 'silver:class_git_pull_requests']
) }}

-- Bitbucket carries no diff totals on the pull request itself, so the per-file
-- diffstat rows are the only source of line counts and are summed here.
WITH diff_stats AS (
    SELECT
        tenant_id,
        source_id,
        repo_full_name,
        pr_id,
        count() AS files_changed,
        sum(lines_added) AS lines_added,
        sum(lines_removed) AS lines_removed,
        max(_airbyte_extracted_at) AS _airbyte_extracted_at,
        1 AS matched
    FROM {{ source('bronze_bitbucket_cloud', 'pull_request_diffstat') }} FINAL
    GROUP BY tenant_id, source_id, repo_full_name, pr_id
),

-- A pull request records no close time of its own; the terminal update event
-- in its activity does.
terminal_activity AS (
    SELECT
        tenant_id,
        source_id,
        repo_full_name,
        pr_id,
        max(event_date) AS closed_on,
        max(_airbyte_extracted_at) AS _airbyte_extracted_at
    FROM {{ source('bronze_bitbucket_cloud', 'pull_request_activity') }} FINAL
    WHERE kind = 'update'
      AND update_state IN ('MERGED', 'DECLINED', 'SUPERSEDED')
    GROUP BY tenant_id, source_id, repo_full_name, pr_id
)

SELECT
    pr.tenant_id AS tenant_id,
    pr.source_id AS source_id,
    pr.unique_key AS unique_key,
    splitByChar('/', COALESCE(pr.repo_full_name, ''))[1] AS project_key,
    splitByChar('/', COALESCE(pr.repo_full_name, ''))[2] AS repo_slug,
    COALESCE(pr.id, 0) AS pr_id,
    COALESCE(pr.id, 0) AS pr_number,
    COALESCE(pr.title, '') AS title,
    COALESCE(pr.description, '') AS description,
    -- A superseded pull request is a declined one to every consumer.
    multiIf(
        pr.state = 'SUPERSEDED', 'DECLINED',
        COALESCE(pr.state, '')
    ) AS state,
    COALESCE(pr.author_display_name, '') AS author_name,
    -- Bitbucket exposes no address on the participant object.
    '' AS author_email,
    COALESCE(pr.source_branch, '') AS source_branch,
    COALESCE(pr.destination_branch, '') AS destination_branch,
    parseDateTimeBestEffortOrNull(pr.created_on) AS created_on,
    parseDateTimeBestEffortOrNull(pr.updated_on) AS updated_on,
    parseDateTimeBestEffortOrNull(
        if(pr.state IN ('MERGED', 'DECLINED', 'SUPERSEDED'), COALESCE(activity.closed_on, ''), '')
    ) AS closed_on,
    COALESCE(pr.merge_commit_sha, '') AS merge_commit_hash,
    -- An unmatched join partner and a genuinely empty pull request both read
    -- as 0 through COALESCE; only the marker separates "not collected yet"
    -- from "changed nothing", and the class columns are nullable to say so.
    if(ds.matched = 1, toNullable(toInt64(COALESCE(ds.files_changed, 0))), NULL) AS files_changed,
    if(ds.matched = 1, toNullable(toInt64(COALESCE(ds.lines_added, 0))), NULL) AS lines_added,
    if(ds.matched = 1, toNullable(toInt64(COALESCE(ds.lines_removed, 0))), NULL) AS lines_removed,
    'insight_bitbucket_cloud' AS data_source,
    toUnixTimestamp64Milli(now64()) AS _version,
    -- Diff stats and activity are their own streams: a late arrival must
    -- re-trigger the pull-request row, which is not re-fetched on its own.
    greatest(
        pr._airbyte_extracted_at,
        COALESCE(ds._airbyte_extracted_at, pr._airbyte_extracted_at),
        COALESCE(activity._airbyte_extracted_at, pr._airbyte_extracted_at)
    ) AS _airbyte_extracted_at
FROM {{ source('bronze_bitbucket_cloud', 'pull_requests') }} AS pr FINAL
LEFT JOIN diff_stats AS ds
    ON ds.tenant_id = pr.tenant_id
    AND ds.source_id = pr.source_id
    AND ds.repo_full_name = pr.repo_full_name
    AND ds.pr_id = pr.id
LEFT JOIN terminal_activity AS activity
    ON activity.tenant_id = pr.tenant_id
    AND activity.source_id = pr.source_id
    AND activity.repo_full_name = pr.repo_full_name
    AND activity.pr_id = pr.id
{% if is_incremental() %}
WHERE greatest(
    pr._airbyte_extracted_at,
    COALESCE(ds._airbyte_extracted_at, pr._airbyte_extracted_at),
    COALESCE(activity._airbyte_extracted_at, pr._airbyte_extracted_at)
) > (SELECT max(_airbyte_extracted_at) FROM {{ this }})
{% endif %}
