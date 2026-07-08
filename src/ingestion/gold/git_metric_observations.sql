{{ config(
    materialized='view',
    schema='insight',
    alias='git_metric_observations',
    tags=['gold']
) }}

-- Source measure observations for the unified metrics runtime, git family.
-- Reads class contracts and the identity bridge only; no vendor-specific
-- columns, tool names, or label mappings may appear in this model. Every
-- measure is emitted through the shape macros in
-- macros/metric_observation_measures.sql; file classification comes from
-- macros/git_file_category.sql (computed here, not in silver, so taxonomy
-- changes apply retroactively).
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
--   PR's linked commits (tie -> unresolved); else the name bridge
--   (identity.git_actor_emails). Unresolved rows are excluded — honest
--   absence, not misattribution.
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

WITH
commits_source AS (
    SELECT
        tenant_id,
        source_id,
        project_key,
        repo_slug,
        commit_hash,
        lower(author_email) AS entity_id,
        toDate(date) AS metric_date,
        lines_added,
        lines_removed,
        replaceOne(data_source, 'insight_', '') AS source_value,
        multiIf(
            source_value = 'github', 'GitHub',
            source_value = 'gitlab', 'GitLab',
            source_value = 'bitbucket_cloud', 'Bitbucket Cloud',
            source_value
        ) AS source_label,
        CAST(
            [tuple('source', source_value, source_label)]
            AS Array(Tuple(key String, value String, label Nullable(String)))
        ) AS source_dimensions
    FROM {{ ref('class_git_commits') }} FINAL
    WHERE author_email != ''
      AND date IS NOT NULL
      AND is_merge_commit = 0
    LIMIT 1 BY tenant_id, data_source, commit_hash
),
file_changes_source AS (
    SELECT
        commits.tenant_id AS tenant_id,
        commits.entity_id AS entity_id,
        commits.metric_date AS metric_date,
        {{ git_file_category('file_changes.file_path') }} AS category,
        {{ git_file_category_label('category') }} AS category_label,
        file_changes.lines_added AS lines_added,
        commits.source_dimensions AS source_dimensions,
        CAST(
            [
                tuple('category', category, category_label),
                tuple('source', commits.source_value, commits.source_label)
            ] AS Array(Tuple(key String, value String, label Nullable(String)))
        ) AS category_source_dimensions
    FROM {{ ref('class_git_file_changes') }} AS file_changes FINAL
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
            lower(commits.author_email) AS email,
            count() AS email_count,
            max(count()) OVER (
                PARTITION BY links.tenant_id, links.source_id,
                             links.project_key, links.repo_slug, links.pr_id
            ) AS max_count
        FROM {{ ref('class_git_pull_requests_commits') }} AS links FINAL
        INNER JOIN {{ ref('class_git_commits') }} AS commits FINAL
            ON commits.tenant_id = links.tenant_id
            AND commits.source_id = links.source_id
            AND commits.project_key = links.project_key
            AND commits.repo_slug = links.repo_slug
            AND commits.commit_hash = links.commit_hash
        WHERE commits.author_email != ''
        GROUP BY tenant_id, source_id, project_key, repo_slug, pr_id, email
    )
    WHERE email_count = max_count
    GROUP BY tenant_id, source_id, project_key, repo_slug, pr_id
),
pull_requests_source AS (
    SELECT
        prs.tenant_id AS tenant_id,
        multiIf(
            prs.author_email != '', lower(prs.author_email),
            pr_commit_emails.email IS NOT NULL AND pr_commit_emails.email != '', pr_commit_emails.email,
            bridge.email IS NOT NULL AND bridge.email != '', bridge.email,
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
        multiIf(
            source_value = 'github', 'GitHub',
            source_value = 'gitlab', 'GitLab',
            source_value = 'bitbucket_cloud', 'Bitbucket Cloud',
            source_value
        ) AS source_label,
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
    LEFT JOIN {{ ref('git_actor_emails') }} AS bridge
        ON bridge.tenant_id = prs.tenant_id
        AND bridge.data_source = prs.data_source
        AND bridge.actor_name = lower(trimBoth(prs.author_name))
    SETTINGS join_use_nulls = 1
),
prs_created_source AS (
    SELECT
        tenant_id,
        assumeNotNull(entity_id) AS entity_id,
        toDate(created_on) AS metric_date,
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
