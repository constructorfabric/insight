{{ config(
    materialized='view',
    schema='insight',
    alias='git_metric_observations',
    tags=['gold']
) }}

-- Source measure observations for the unified metrics runtime, git family.
-- Reads class contracts only; no vendor-specific columns or tool names may
-- appear inline. Every measure is emitted through the shape macros in
-- macros/metric_observation_measures.sql; file classification and the
-- source-dimension display label come from macros/git_file_category.sql
-- (static product vocabulary, computed here rather than in silver so
-- taxonomy/label changes apply retroactively on every read).
--
-- Grain per measure:
--   day-grain sums:  commit_count, code_lines_added, lines_added,
--                    pr_created, pr_merged
--   day-grain presence: commit_day
--   event-grain (one row per source event, feeding median metrics):
--                    commit_change_size (per non-merge commit),
--                    pr_cycle_hours (per merged pull request),
--                    pr_change_size (per pull request)
--
-- Attribution:
--   Commits and file changes attribute by the commit author_email.
--   Pull requests resolve in tiers, never guessing: the PR's own
--   author_email when present; else the dominant author_email among the
--   PR's linked commits (tie -> unresolved). Rows that resolve to no
--   email are excluded — honest absence, never a name-matched guess.
--
-- Dating:
--   pr_created anchors at created_on. pr_merged and pr_cycle_hours anchor
--   at closed_on gated on state MERGED: the sources set the close
--   timestamp to (or coalesce it with) the merge timestamp for merged
--   pull requests, and a merge-commit join would risk fan-out for
--   marginal precision. Negative durations (dirty close timestamps) are
--   excluded from cycle hours.
--
-- Merge commits are excluded once in commits_source; the exclusion
-- propagates to file-change measures through the authorship join.
-- `LIMIT 1 BY` collapses the same commit hash appearing in more than one
-- repo of a source (forks), keeping commit_count a distinct-hash count.
--
-- Memory shape (this view executes live on every metric query, and its
-- measure branches run concurrently within one query, so per-branch memory
-- multiplies): FINAL is the cheapest dedup here — a streaming merge of
-- sorted parts — and stays wherever dedup is needed. Version-ordered
-- `ORDER BY .. LIMIT 1 BY` is not an alternative: it buffers a full sort
-- of the read (measured ~2x the memory of FINAL at scale). The two reads
-- that avoid FINAL do so because they need no dedup at all: the identity
-- vote in pr_commit_emails aggregates by uniqExact, which duplicate row
-- versions cannot inflate. file_changes pre-aggregates to commit x category
-- grain before joining, so per-file rows and file_path strings never enter
-- a join side or a measure aggregation.

WITH
commits_source AS (
    SELECT
        tenant_id,
        source_id,
        project_key,
        repo_slug,
        commit_hash,
        -- Match the API's entity-id normalization exactly (trim + lower):
        -- observations keyed on an untrimmed id would never be matched by a
        -- request the frontend normalizes.
        lower(trimBoth(author_email)) AS entity_id,
        toDate(date) AS metric_date,
        lines_added,
        lines_removed,
        replaceOne(data_source, 'insight_', '') AS source_value,
        {{ git_source_label('source_value') }} AS source_label,
        CAST(
            [tuple('source', source_value, source_label)]
            AS Array(Tuple(key String, value String, label Nullable(String)))
        ) AS source_dimensions
    FROM {{ ref('class_git_commits') }} FINAL
    WHERE trimBoth(author_email) != ''
      AND date IS NOT NULL
      AND is_merge_commit = 0
    -- Deterministic survivor per (tenant, source, hash): without an ORDER BY,
    -- LIMIT 1 BY picks an arbitrary repo copy of a forked commit, which would
    -- vary across runs and shift the project_key/repo_slug used by the
    -- file-change join.
    ORDER BY tenant_id, data_source, commit_hash, source_id, project_key, repo_slug
    LIMIT 1 BY tenant_id, data_source, commit_hash
),
file_changes_source AS (
    SELECT
        commits.tenant_id AS tenant_id,
        commits.entity_id AS entity_id,
        commits.metric_date AS metric_date,
        file_changes.category AS category,
        {{ git_file_category_label('file_changes.category') }} AS category_label,
        file_changes.lines_added AS lines_added,
        commits.source_dimensions AS source_dimensions,
        CAST(
            [
                tuple('category', category, category_label),
                tuple('source', commits.source_value, commits.source_label)
            ] AS Array(Tuple(key String, value String, label Nullable(String)))
        ) AS category_source_dimensions
    FROM (
        -- Aggregated to commit x category grain before the join, so per-file
        -- rows and file_path strings never reach the join or the measure
        -- aggregations.
        SELECT
            tenant_id,
            source_id,
            project_key,
            repo_slug,
            commit_hash,
            {{ git_file_category('file_path') }} AS category,
            sum(lines_added) AS lines_added
        FROM {{ ref('class_git_file_changes') }} FINAL
        GROUP BY tenant_id, source_id, project_key, repo_slug, commit_hash, category
    ) AS file_changes
    INNER JOIN commits_source AS commits
        ON commits.tenant_id = file_changes.tenant_id
        AND commits.source_id = file_changes.source_id
        AND commits.project_key = file_changes.project_key
        AND commits.repo_slug = file_changes.repo_slug
        AND commits.commit_hash = file_changes.commit_hash
),
pr_commit_emails AS (
    -- Dominant commit author email per pull request (tie -> NULL): the
    -- strongest identity signal for PR authors whose source hides emails.
    SELECT
        tenant_id,
        source_id,
        project_key,
        repo_slug,
        pr_id,
        if(uniqExact(email) = 1, any(email), CAST(NULL AS Nullable(String))) AS email
    FROM (
        SELECT
            links.tenant_id AS tenant_id,
            links.source_id AS source_id,
            links.project_key AS project_key,
            links.repo_slug AS repo_slug,
            links.pr_id AS pr_id,
            lower(trimBoth(commits.author_email)) AS email,
            -- Vote by distinct linked commits, not join rows: a hash present
            -- in more than one repo of the source must not double-count.
            uniqExact(commits.commit_hash) AS email_count,
            max(uniqExact(commits.commit_hash)) OVER (
                PARTITION BY links.tenant_id, links.source_id,
                             links.project_key, links.repo_slug, links.pr_id
            ) AS max_count
        -- No dedup on either side: duplicate versions of a link or commit
        -- row cannot inflate the uniqExact(commit_hash) vote below.
        FROM {{ ref('class_git_pull_requests_commits') }} AS links
        INNER JOIN {{ ref('class_git_commits') }} AS commits
            ON commits.tenant_id = links.tenant_id
            AND commits.source_id = links.source_id
            AND commits.project_key = links.project_key
            AND commits.repo_slug = links.repo_slug
            AND commits.commit_hash = links.commit_hash
        -- Non-merge authorship only, matching the commit observations; a merge
        -- commit's author should not vote in the PR's identity election.
        WHERE trimBoth(commits.author_email) != ''
          AND commits.is_merge_commit = 0
        GROUP BY tenant_id, source_id, project_key, repo_slug, pr_id, email
    )
    WHERE email_count = max_count
    GROUP BY tenant_id, source_id, project_key, repo_slug, pr_id
),
pull_requests_source AS (
    SELECT
        prs.tenant_id AS tenant_id,
        multiIf(
            trimBoth(prs.author_email) != '', lower(trimBoth(prs.author_email)),
            pr_commit_emails.email IS NOT NULL AND pr_commit_emails.email != '', pr_commit_emails.email,
            CAST(NULL AS Nullable(String))
        ) AS entity_id,
        prs.state AS state,
        prs.created_on AS created_on,
        prs.closed_on AS closed_on,
        prs.lines_added + prs.lines_removed AS change_size,
        if(
            prs.state = 'MERGED'
                AND prs.closed_on IS NOT NULL
                AND prs.created_on IS NOT NULL
                AND prs.closed_on >= prs.created_on,
            dateDiff('second', prs.created_on, prs.closed_on) / 3600.0,
            CAST(NULL AS Nullable(Float64))
        ) AS cycle_hours,
        replaceOne(prs.data_source, 'insight_', '') AS source_value,
        {{ git_source_label('source_value') }} AS source_label,
        CAST(
            [tuple('source', source_value, source_label)]
            AS Array(Tuple(key String, value String, label Nullable(String)))
        ) AS source_dimensions
    FROM {{ ref('class_git_pull_requests') }} AS prs FINAL
    LEFT JOIN pr_commit_emails
        ON pr_commit_emails.tenant_id = prs.tenant_id
        AND pr_commit_emails.source_id = prs.source_id
        AND pr_commit_emails.project_key = prs.project_key
        AND pr_commit_emails.repo_slug = prs.repo_slug
        AND pr_commit_emails.pr_id = prs.pr_id
    SETTINGS join_use_nulls = 1
),
prs_created_source AS (
    SELECT
        tenant_id,
        assumeNotNull(entity_id) AS entity_id,
        toDate(created_on) AS metric_date,
        state,
        change_size,
        source_dimensions
    FROM pull_requests_source
    WHERE entity_id IS NOT NULL
      AND entity_id != ''
      AND created_on IS NOT NULL
),
prs_merged_source AS (
    SELECT
        tenant_id,
        assumeNotNull(entity_id) AS entity_id,
        toDate(closed_on) AS metric_date,
        cycle_hours,
        source_dimensions
    FROM pull_requests_source
    WHERE entity_id IS NOT NULL
      AND entity_id != ''
      AND state = 'MERGED'
      AND closed_on IS NOT NULL
),
measure_observations AS (
    {{ sum_measure('commit_count', 'commits_source', '1', 'source_dimensions') }}

    UNION ALL

    {{ presence_measure('commit_day', ['commits_source']) }}

    UNION ALL

    {{ event_measure('commit_change_size', 'commits_source', 'lines_added + lines_removed', 'source_dimensions') }}

    UNION ALL

    {{ sum_measure('code_lines_added', 'file_changes_source', 'lines_added', 'source_dimensions', where="category = 'code'") }}

    UNION ALL

    {{ sum_measure('lines_added', 'file_changes_source', 'lines_added', 'category_source_dimensions') }}

    UNION ALL

    {{ sum_measure('pr_created', 'prs_created_source', '1', 'source_dimensions') }}

    UNION ALL

    -- Merge-rate numerator: PRs *created* in the period that have merged,
    -- dated at creation so numerator and denominator share the created-cohort.
    -- (pr_merged below is merge-dated throughput for the standalone metric.)
    {{ sum_measure('pr_created_merged', 'prs_created_source', '1', 'source_dimensions', where="state = 'MERGED'") }}

    UNION ALL

    {{ sum_measure('pr_merged', 'prs_merged_source', '1', 'source_dimensions') }}

    UNION ALL

    {{ event_measure('pr_cycle_hours', 'prs_merged_source', 'cycle_hours', 'source_dimensions') }}

    UNION ALL

    {{ event_measure('pr_change_size', 'prs_created_source', 'change_size', 'source_dimensions', where='change_size > 0') }}
)
SELECT
    assumeNotNull(tenant_id) AS tenant_id,
    'git' AS source_key,
    'person' AS entity_type,
    assumeNotNull(entity_id) AS entity_id,
    assumeNotNull(metric_date) AS metric_date,
    CAST(NULL AS Nullable(DateTime64(3))) AS observed_at,
    measure_key,
    value,
    CAST(NULL AS Nullable(String)) AS subject_key,
    dimensions
FROM measure_observations
WHERE tenant_id IS NOT NULL
  AND entity_id IS NOT NULL
  AND metric_date IS NOT NULL
