{{ config(
    materialized='table',
    engine='MergeTree',
    order_by=['tenant_id', 'author_email', 'authored_at'],
    partition_by='toYYYYMM(authored_date)',
    schema=var('gold_database'),
    settings={'allow_nullable_key': 1},
    tags=['gold'],
    query_settings=metric_serving_query_settings()
) }}

-- Authored commits: one row per commit a person wrote, with the lines that
-- commit contributed.
--
-- INVARIANT: a projection of the shared commit stage, so metric evidence and
-- drilldown read the same derivation.

WITH
reported_commit_file_lines AS (
    SELECT
        tenant_id,
        source_id,
        project_key,
        repo_slug,
        commit_hash,
        sum(lines_added) AS lines_added,
        sum(lines_removed) AS lines_removed
    FROM {{ ref('git_commit_file_changes') }}
    GROUP BY tenant_id, source_id, project_key, repo_slug, commit_hash
),
authored_commit_file_lines AS (
    SELECT
        tenant_id,
        source_id,
        project_key,
        repo_slug,
        commit_hash,
        sum(lines_added) AS lines_added,
        sum(lines_removed) AS lines_removed
    FROM {{ ref('git_file_changes') }}
    GROUP BY tenant_id, source_id, project_key, repo_slug, commit_hash
)

SELECT
    commits.tenant_id AS tenant_id,
    commits.source_id AS source_id,
    commits.project_key AS project_key,
    commits.repo_slug AS repo_slug,
    commits.commit_hash AS commit_hash,
    commits.entity_id AS author_email,
    commits.author_name AS author_name,
    commits.message AS message,
    -- SAFETY: the stage admits no row without a date, and neither a partition
    -- key nor a sort key may be nullable.
    assumeNotNull(commits.observed_at) AS authored_at,
    assumeNotNull(commits.metric_date) AS authored_date,
    commits.branch_scope_value AS branch_scope,
    commits.branch_scope_label AS branch_scope_label,
    commits.repository_value AS repository,
    commits.repository_label AS repository_label,
    commits.project_value AS project,
    commits.project_label AS project_label,
    commits.source_value AS source,
    commits.source_label AS source_label,
    -- A commit's own stats, less the lines of the file changes that lost the
    -- content dedup, so a commit that introduces nothing new reports zero.
    --
    -- SAFETY: the NULL check is explicit because `greatest` IGNORES NULL, and
    -- the floor covers stats that disagree with the summed file changes.
    if(
        commits.lines_added IS NULL,
        CAST(NULL AS Nullable(Int64)),
        toNullable(greatest(
            toInt64(0),
            assumeNotNull(commits.lines_added)
                - (coalesce(reported.lines_added, 0) - coalesce(authored.lines_added, 0))
        ))
    ) AS lines_added,
    if(
        commits.lines_removed IS NULL,
        CAST(NULL AS Nullable(Int64)),
        toNullable(greatest(
            toInt64(0),
            assumeNotNull(commits.lines_removed)
                - (coalesce(reported.lines_removed, 0) - coalesce(authored.lines_removed, 0))
        ))
    ) AS lines_removed
FROM {{ ref('git_authored_commits') }} AS commits
LEFT JOIN reported_commit_file_lines AS reported
    ON reported.tenant_id = commits.tenant_id
    AND reported.source_id = commits.source_id
    AND reported.project_key = commits.project_key
    AND reported.repo_slug = commits.repo_slug
    AND reported.commit_hash = commits.commit_hash
LEFT JOIN authored_commit_file_lines AS authored
    ON authored.tenant_id = commits.tenant_id
    AND authored.source_id = commits.source_id
    AND authored.project_key = commits.project_key
    AND authored.repo_slug = commits.repo_slug
    AND authored.commit_hash = commits.commit_hash
