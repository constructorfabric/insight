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

-- Authored commits: one row per commit a person wrote, carrying the lines that
-- commit actually contributed. A semantic-layer dataset — measures aggregate
-- it, drilldown projects it, and neither re-solves what is settled upstream:
-- which commits are one commit, whose lines are whose, and whether the work
-- reached the default branch.
--
-- A projection of the shared commit stage onto this dataset's column names, so
-- the derivation the metric evidence reads and the derivation a drilldown
-- reads are the same derivation.
--
-- Ordered by tenant, person and time because that is how it is read: one
-- person's commits over a period, never a scan of every commit ever made.

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
    -- A commit's own line stats, less the lines of the file changes that lost
    -- the content dedup. The stats stay the base — a source can report a
    -- commit's totals without reporting its file changes at all — and only what
    -- the dedup removed is taken back out, so a commit that introduces nothing
    -- new reports a size of zero and its file rows agree with what it
    -- contributed.
    --
    -- SAFETY: the NULL check is explicit because `greatest` IGNORES NULL
    -- arguments — `greatest(0, NULL)` is 0, which would invent a size for a
    -- commit whose source reported no line stats. `greatest` floors the result
    -- because a commit's own stats and the sum of its file changes need not
    -- agree (binary files, truncated diffs).
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
