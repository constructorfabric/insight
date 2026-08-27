{{ config(
    materialized='table',
    engine='MergeTree',
    order_by=['data_source'],
    schema=var('gold_database'),
    settings={'allow_nullable_key': 1},
    tags=['gold'],
    query_settings=metric_serving_query_settings(join_use_nulls=1)
) }}

-- How much of a source's authored work arrived with its file changes. One row
-- per git source: commits counted, commits the file-change stream reached, and
-- the size the rest report from their commit rows alone.
--
-- This is the measuring device for diff collection, and it exists because the
-- size metrics no longer are one. A commit whose changes were never collected
-- used to be visible as a commit with no lines beside it — the discrepancy
-- that surfaced a connector collecting file changes for the default branch
-- only. The totals now report that commit's own size, which is the right
-- number to serve and the wrong place to keep a collection alarm.
--
-- Read `recent_collected_pct`, not `collected_pct`, to judge a live source.
-- The all-time figure carries history no re-read can reach, so it is a floor
-- that never fully recovers; the recent window is the one a regression moves.
-- `uncollected_lines` is the prioritisation signal — how much authored volume
-- is described by a commit row and nothing finer.
--
-- INVARIANT: join_use_nulls=1 is what makes the membership test mean anything.
-- file_change_rows is a count, so under the default setting an unmatched LEFT
-- JOIN would fill it with 0 rather than NULL and every commit would read as
-- collected.

WITH authored AS (
    SELECT
        commits.data_source AS data_source,
        commits.metric_date AS metric_date,
        commits.lines_added AS lines_added,
        commits.lines_removed AS lines_removed,
        collected.file_change_rows AS file_change_rows
    FROM {{ ref('git_authored_commits') }} AS commits
    LEFT JOIN {{ ref('git_commit_file_line_totals') }} AS collected
        ON collected.tenant_id = commits.tenant_id
        AND collected.source_id = commits.source_id
        AND collected.project_key = commits.project_key
        AND collected.repo_slug = commits.repo_slug
        AND collected.commit_hash = commits.commit_hash
)
SELECT
    data_source,
    count() AS commits,
    countIf(file_change_rows IS NOT NULL) AS commits_with_file_changes,
    round(100 * countIf(file_change_rows IS NOT NULL) / count(), 1) AS collected_pct,
    countIf(metric_date >= today() - {{ var('git_coverage_recent_days') }}) AS recent_commits,
    round(
        100 * countIf(
            file_change_rows IS NOT NULL
                AND metric_date >= today() - {{ var('git_coverage_recent_days') }}
        ) / nullIf(countIf(metric_date >= today() - {{ var('git_coverage_recent_days') }}), 0),
        1
    ) AS recent_collected_pct,
    sumIf(
        coalesce(lines_added, 0) + coalesce(lines_removed, 0),
        file_change_rows IS NULL
    ) AS uncollected_lines
FROM authored
GROUP BY data_source
