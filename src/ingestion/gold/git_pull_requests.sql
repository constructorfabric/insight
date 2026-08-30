{{ config(
    materialized='table',
    engine='MergeTree',
    order_by=['tenant_id', 'author_email', 'created_at'],
    partition_by='toYYYYMM(created_date)',
    schema=var('gold_database'),
    settings={'allow_nullable_key': 1},
    tags=['gold'],
    query_settings=metric_serving_query_settings()
) }}

-- Opened pull requests: one row per request a person raised, with the review it
-- received and the waits it went through.
--
-- INVARIANT: identity is carried in both forms the source offers — the author's
-- profile email and the author's account id — and nothing here derives an
-- identity the source did not state.

WITH
-- The default branch NAME per repository. min() rather than any() so a
-- repository claiming two default branches resolves the same way on every read.
repository_default_branches AS (
    SELECT
        tenant_id,
        source_id,
        project_key,
        repo_slug,
        min(branch_name) AS branch_name
    FROM {{ ref('class_git_repository_branches') }} FINAL
    WHERE is_default = 1
    GROUP BY tenant_id, source_id, project_key, repo_slug
),
-- A reviewer counts once and only with evidence of a review, so a reviewer
-- merely requested does not read as one who looked.
--
-- SAFETY: an approval may carry no review time, so a NULL `approved_at` is not
-- proof that nobody approved.
pull_request_review_summary AS (
    SELECT
        tenant_id,
        source_id,
        project_key,
        repo_slug,
        pr_id,
        uniqExactIf(
            reviewer_uuid,
            reviewer_uuid != '' AND (reviewed_at IS NOT NULL OR approved = 1)
        ) AS reviewer_count,
        max(approved) AS has_approval,
        minIfOrNull(reviewed_at, reviewed_at IS NOT NULL) AS first_reviewed_at,
        maxIfOrNull(reviewed_at, approved = 1 AND reviewed_at IS NOT NULL) AS approved_at
    FROM {{ ref('class_git_pull_requests_reviewers') }} FINAL
    GROUP BY tenant_id, source_id, project_key, repo_slug, pr_id
),
-- uniqExact, not count(): the link table is append-only per sync, so one link
-- can arrive more than once and the count must not inflate.
--
-- INVARIANT: toNullable fixes the column's type whatever `join_use_nulls` is,
-- and keeps a request the source linked nothing for NULL rather than a merge
-- of zero commits.
pull_request_commit_counts AS (
    SELECT
        tenant_id,
        source_id,
        project_key,
        repo_slug,
        pr_id,
        toNullable(uniqExact(commit_hash)) AS linked_commit_count
    FROM {{ ref('class_git_pull_requests_commits') }} FINAL
    GROUP BY tenant_id, source_id, project_key, repo_slug, pr_id
)

SELECT
    prs.tenant_id AS tenant_id,
    prs.source_id AS source_id,
    prs.project_key AS project_key,
    prs.repo_slug AS repo_slug,
    prs.pr_id AS pr_id,
    prs.pr_number AS pr_number,
    prs.title AS title,
    -- '' rather than a dropped row: a request whose author the source names only
    -- by account still belongs to that person.
    lower(trimBoth(prs.author_email)) AS author_email,
    prs.author_account_id AS author_account_id,
    prs.author_name AS author_name,
    -- SAFETY: safe under the WHERE below, and a partition key may not be
    -- nullable.
    assumeNotNull(prs.created_on) AS created_at,
    toDate(assumeNotNull(prs.created_on)) AS created_date,
    prs.closed_on AS closed_at,
    (prs.state = 'MERGED') AS merged,
    (prs.closed_on IS NOT NULL AND prs.state != 'MERGED') AS abandoned,
    (
        prs.state = 'MERGED'
            AND prs.closed_on IS NOT NULL
            AND coalesce(review_summary.has_approval, 0) = 0
    ) AS merged_without_approval,
    coalesce(review_summary.reviewer_count, 0) AS reviewer_count,
    review_summary.first_reviewed_at AS first_reviewed_at,
    review_summary.approved_at AS approved_at,
    -- INVARIANT: an unreported destination and an unknown default branch both
    -- read `non_default`, which keeps default + non_default = total.
    if(
        prs.destination_branch != ''
            AND prs.destination_branch = coalesce(defaults.branch_name, ''),
        'default',
        'non_default'
    ) AS branch_scope,
    {{ git_branch_scope_label('branch_scope') }} AS branch_scope_label,
    if(prs.destination_branch = '', '__unknown__', prs.destination_branch) AS destination_branch,
    if(prs.destination_branch = '', 'Unknown', prs.destination_branch) AS destination_branch_label,
    concat(coalesce(toString(prs.source_id), ''), ':', coalesce(prs.project_key, ''), '/', coalesce(prs.repo_slug, '')) AS repository,
    if(coalesce(prs.project_key, '') = '', coalesce(prs.repo_slug, ''), concat(prs.project_key, '/', prs.repo_slug)) AS repository_label,
    if(coalesce(prs.project_key, '') = '', '__unknown__', concat(coalesce(toString(prs.source_id), ''), ':', prs.project_key)) AS project,
    if(coalesce(prs.project_key, '') = '', 'Unknown', prs.project_key) AS project_label,
    replaceOne(prs.data_source, 'insight_', '') AS source,
    {{ git_source_label('source') }} AS source_label,
    prs.lines_added AS lines_added,
    prs.lines_removed AS lines_removed,
    prs.files_changed AS files_changed,
    commit_counts.linked_commit_count AS linked_commit_count,
    -- INVARIANT: every duration is NULL unless its pair of timestamps exists and
    -- runs forwards; a negative span is a source disagreeing with its own clock.
    if(
        prs.state = 'MERGED'
            AND prs.closed_on IS NOT NULL
            AND prs.closed_on >= prs.created_on,
        dateDiff('second', prs.created_on, prs.closed_on) / 3600.0,
        CAST(NULL AS Nullable(Float64))
    ) AS cycle_hours,
    if(
        review_summary.first_reviewed_at IS NOT NULL
            AND review_summary.first_reviewed_at >= prs.created_on,
        dateDiff('second', prs.created_on, review_summary.first_reviewed_at) / 3600.0,
        CAST(NULL AS Nullable(Float64))
    ) AS first_review_hours,
    if(
        prs.state = 'MERGED'
            AND prs.closed_on IS NOT NULL
            AND review_summary.first_reviewed_at IS NOT NULL
            AND prs.closed_on >= review_summary.first_reviewed_at,
        dateDiff('second', review_summary.first_reviewed_at, prs.closed_on) / 3600.0,
        CAST(NULL AS Nullable(Float64))
    ) AS review_to_merge_hours,
    if(
        prs.state = 'MERGED'
            AND prs.closed_on IS NOT NULL
            AND review_summary.approved_at IS NOT NULL
            AND prs.closed_on >= review_summary.approved_at,
        dateDiff('second', review_summary.approved_at, prs.closed_on) / 3600.0,
        CAST(NULL AS Nullable(Float64))
    ) AS approval_to_merge_hours,
    -- Both legs of the split must exist for the share to mean anything: a
    -- request reviewed but not merged has a wait with nothing to be a share of.
    if(
        first_review_hours IS NOT NULL
            AND review_to_merge_hours IS NOT NULL
            AND cycle_hours IS NOT NULL
            AND cycle_hours > 0,
        100.0 * assumeNotNull(first_review_hours) / assumeNotNull(cycle_hours),
        CAST(NULL AS Nullable(Float64))
    ) AS review_wait_share
FROM {{ ref('class_git_pull_requests') }} AS prs FINAL
LEFT JOIN pull_request_review_summary AS review_summary
    ON review_summary.tenant_id = prs.tenant_id
    AND review_summary.source_id = prs.source_id
    AND review_summary.project_key = prs.project_key
    AND review_summary.repo_slug = prs.repo_slug
    AND review_summary.pr_id = prs.pr_id
LEFT JOIN pull_request_commit_counts AS commit_counts
    ON commit_counts.tenant_id = prs.tenant_id
    AND commit_counts.source_id = prs.source_id
    AND commit_counts.project_key = prs.project_key
    AND commit_counts.repo_slug = prs.repo_slug
    AND commit_counts.pr_id = prs.pr_id
LEFT JOIN repository_default_branches AS defaults
    ON defaults.tenant_id = prs.tenant_id
    AND defaults.source_id = prs.source_id
    AND defaults.project_key = prs.project_key
    AND defaults.repo_slug = prs.repo_slug
WHERE prs.created_on IS NOT NULL
  AND (
      lower(trimBoth(prs.author_email)) != ''
      OR coalesce(prs.author_account_id, '') != ''
  )
