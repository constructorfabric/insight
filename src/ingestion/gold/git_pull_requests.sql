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

-- Opened pull requests: one row per request a person raised, carrying the
-- review it received and the waits it went through. A semantic-layer dataset —
-- measures aggregate it, drilldown projects it, and neither re-solves what is
-- settled here: whose request it is, whether it targets the default branch,
-- and how long each stage took.
--
-- Identity stays as the source gave it, in BOTH forms the source offers: the
-- author's profile email and the author's account id. The account is the
-- source's own answer to "whose request is this" and survives an empty profile
-- email, so a query binds the person from it first and from the email only when
-- no account is bound. Nothing here derives an identity the source did not
-- state.
--
-- Ordered by tenant, person and time because that is how it is read: one
-- person's requests over a period, never a scan of every request ever opened.

WITH
-- The default branch NAME per repository, which is what a request's
-- destination has to be compared against. Every git connector reports it.
-- min() rather than any() so a repository that somehow claims two default
-- branches resolves the same way on every read.
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
-- What review a request received. A reviewer counts once and only with evidence
-- of a review — a recorded review time, or an approval — so a reviewer merely
-- requested does not read as one who looked.
--
-- SAFETY: an approval may carry no review time, so `has_approval` and
-- `approved_at` are separate facts: a NULL `approved_at` is not proof that
-- nobody approved.
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
    -- by account still belongs to that person, and the account binds it when a
    -- query runs.
    lower(trimBoth(prs.author_email)) AS author_email,
    prs.author_account_id AS author_account_id,
    prs.author_name AS author_name,
    -- assumeNotNull is safe under the WHERE below, and keeps the event time and
    -- the partition key out of Nullable: a partition key may not be nullable,
    -- and a sort key that is costs a read for nothing.
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
    -- A request targets the default branch or it does not. An unreported
    -- destination, and a repository whose default branch is unknown, both read
    -- `non_default` — the agreed reading for an absent signal, which keeps
    -- default + non_default = total.
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
    -- Every duration is NULL unless the pair of timestamps it spans exists and
    -- runs forwards. A negative span is a source disagreeing with itself about
    -- its own clock, and averaging it in would move a team's number by a value
    -- that never happened.
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
    -- The share of a merged request's life spent waiting for its first look.
    -- Both legs of the split have to exist for the share to mean anything: a
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
