{{ config(
    materialized='table',
    engine='MergeTree',
    order_by=['tenant_id', 'data_source', 'source_id'],
    schema=var('gold_database'),
    settings={'allow_nullable_key': 1},
    tags=['gold'],
    query_settings=metric_serving_query_settings(join_use_nulls=1)
) }}

-- How much of each connector instance's authored work arrived with its file
-- changes. The measuring device for diff collection, and it exists because the
-- size metrics no longer are one: a commit whose changes were never collected
-- used to be visible as a commit with no lines beside it — the discrepancy
-- that surfaced a connector collecting file changes for the default branch
-- only. The totals now report that commit's own size, which is the right
-- number to serve and the wrong place to keep a collection alarm.
--
-- Measured on the CLASS relations, not the gold commit set, and grouped per
-- source_id: collection is a property of a connector instance, and by the time
-- a commit reaches git_authored_commits the same hash from a fork and its
-- upstream is deliberately collapsed to one row carrying whichever source
-- sorted first. A healthy instance would hide a broken one.
--
-- Three size classes, because "no file change" is not one condition:
--   * requires — the source reported a non-zero size, so a diff must exist.
--     The only class coverage is measured over.
--   * known_zero — the source reported 0/0. No file detail was ever due, and
--     counting it as a loss would move coverage with the share of empty
--     commits rather than with collection health.
--   * unknown — the source reported no size at all. Neither success nor
--     failure, and a data-quality signal of its own.
--
-- Merge commits are out entirely: every connector skips their diffs by design
-- (`parent_count <= 1` on the walk), so they are not a collection failure.
--
-- Judge a live instance by recent_collected_pct. The all-time share carries
-- history no re-read can reach, so it is a floor that never fully recovers.
--
-- INVARIANT: join_use_nulls=1 is what makes the membership test mean anything.
-- file_change_rows is a count, so under the default setting an unmatched LEFT
-- JOIN would fill it with 0 rather than NULL and every commit would read as
-- collected. tenant_id and source_id are Nullable on the class, so they join
-- null-safe — a plain `=` never matches NULL to NULL and would report a whole
-- instance as uncollected.
--
-- INVARIANT: the membership key carries data_source. source_id is Nullable, so
-- two providers whose rows both leave it NULL join null-safe to each other on
-- shared repository coordinates, and one provider's file change would answer
-- for another provider's commit — the hiding this model is grouped to prevent.

WITH collected AS (
    SELECT
        tenant_id,
        data_source,
        source_id,
        project_key,
        repo_slug,
        commit_hash,
        count() AS file_change_rows
    FROM {{ ref('class_git_file_changes') }} FINAL
    GROUP BY tenant_id, data_source, source_id, project_key, repo_slug, commit_hash
),
classified AS (
    SELECT
        commits.tenant_id AS tenant_id,
        commits.data_source AS data_source,
        commits.source_id AS source_id,
        toDate(commits.date) AS commit_date,
        multiIf(
            coalesce(commits.lines_added, 0) > 0 OR coalesce(commits.lines_removed, 0) > 0,
            'requires',
            commits.lines_added IS NOT NULL AND commits.lines_removed IS NOT NULL,
            'known_zero',
            'unknown'
        ) AS size_class,
        coalesce(commits.lines_added, 0) + coalesce(commits.lines_removed, 0) AS reported_size,
        collected.file_change_rows IS NOT NULL AS has_file_changes
    FROM {{ ref('class_git_commits') }} AS commits FINAL
    LEFT JOIN collected
        ON collected.tenant_id IS NOT DISTINCT FROM commits.tenant_id
        AND collected.data_source = commits.data_source
        AND collected.source_id IS NOT DISTINCT FROM commits.source_id
        AND collected.project_key = commits.project_key
        AND collected.repo_slug = commits.repo_slug
        AND collected.commit_hash = commits.commit_hash
    WHERE commits.is_merge_commit = 0
      AND commits.date IS NOT NULL
)
SELECT
    tenant_id,
    data_source,
    source_id,
    count() AS commits,
    countIf(size_class = 'requires') AS commits_requiring_file_changes,
    countIf(size_class = 'known_zero') AS known_zero_size_commits,
    countIf(size_class = 'unknown') AS unknown_size_commits,
    countIf(size_class = 'requires' AND has_file_changes) AS commits_with_file_changes,
    round(
        100 * countIf(size_class = 'requires' AND has_file_changes)
            / nullIf(countIf(size_class = 'requires'), 0),
        1
    ) AS collected_pct,
    countIf(
        size_class = 'requires'
            AND commit_date >= today() - {{ var('git_coverage_recent_days') }}
    ) AS recent_commits_requiring_file_changes,
    round(
        100 * countIf(
            size_class = 'requires'
                AND has_file_changes
                AND commit_date >= today() - {{ var('git_coverage_recent_days') }}
        ) / nullIf(
            countIf(
                size_class = 'requires'
                    AND commit_date >= today() - {{ var('git_coverage_recent_days') }}
            ),
            0
        ),
        1
    ) AS recent_collected_pct,
    sumIf(reported_size, size_class = 'requires' AND NOT has_file_changes) AS uncollected_lines
FROM classified
GROUP BY tenant_id, data_source, source_id
