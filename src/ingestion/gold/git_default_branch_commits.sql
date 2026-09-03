{{ config(
    materialized='table',
    engine='MergeTree',
    order_by=['tenant_id', 'source_id', 'project_key', 'repo_slug', 'commit_hash'],
    schema=var('gold_database'),
    settings={'allow_nullable_key': 1},
    tags=['gold'],
    query_settings=metric_serving_query_settings()
) }}

-- Commits that reached the default branch through a merged pull request.
-- Materialized so the PR-membership joins run once per build in their own
-- query budget instead of inside the evidence build.
--
-- `is_default_branch` alone is reachability AT SYNC TIME: a commit first seen
-- on a feature branch stays 0, and only a re-read corrects it. That is the
-- wrong tense for a `branch_scope` that means "did this work land". A merged
-- request whose destination IS the default branch is standing proof that its
-- commits landed, and it is proof the sources keep indefinitely.
--
-- A squash merged with NO pull request has no membership edge to read, and the
-- squash rewrites history, so the originals are unreachable from the default
-- branch for good. The second half of this model reads the content instead: a
-- change whose object id sits on a default-branch commit at the same path
-- landed, whichever commit carries it there.
--
-- INVARIANT: content can only heal what SURVIVED into the default branch. A
-- path a branch touched more than once keeps its intermediate versions
-- nowhere but the branch, so those commits stay `non_default` — a squash
-- carries the span's end, not its steps.
--
-- INVARIANT: this cannot see a branch merged by a fast-forward push with no
-- pull request. GitLab still corrects those itself (advancing the default head
-- re-walks the range), the proxy-backed sources only within their lookback
-- window.
WITH
-- The default branch NAME per repository, which is what a pull request's
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

-- The content that reached the default branch, by path. Object ids only: a
-- source that reports none says nothing about where its content is, and an
-- empty identity would match every such row to every other.
landed_content AS (
    SELECT DISTINCT
        landed_change.tenant_id AS tenant_id,
        landed_change.source_id AS source_id,
        landed_change.project_key AS project_key,
        landed_change.repo_slug AS repo_slug,
        landed_change.file_path AS file_path,
        {{ git_file_content_identity('landed_change.post_image_oid', 'landed_change.pre_image_oid') }} AS content_identity
    FROM {{ ref('class_git_file_changes') }} AS landed_change FINAL
    WHERE (landed_change.tenant_id, landed_change.source_id, landed_change.project_key, landed_change.repo_slug, landed_change.commit_hash) IN (
        SELECT tenant_id, source_id, project_key, repo_slug, commit_hash
        FROM {{ ref('class_git_commits') }} FINAL
        WHERE is_default_branch = 1
    )
      AND NOT (
          coalesce(landed_change.pre_image_oid, '') = ''
              AND coalesce(landed_change.post_image_oid, '') = ''
      )
),
-- The candidates: commits the default branch does not reach. Narrowing here
-- rather than filtering afterwards is what keeps a commit from matching its
-- own content and reporting itself as healed.
unlanded_commits AS (
    SELECT
        tenant_id,
        source_id,
        project_key,
        repo_slug,
        commit_hash
    FROM {{ ref('class_git_commits') }} FINAL
    WHERE coalesce(is_default_branch, 0) != 1
      AND is_merge_commit = 0
)
SELECT DISTINCT
    tenant_id,
    source_id,
    project_key,
    repo_slug,
    commit_hash
FROM (
    SELECT
        links.tenant_id AS tenant_id,
        links.source_id AS source_id,
        links.project_key AS project_key,
        links.repo_slug AS repo_slug,
        links.commit_hash AS commit_hash
    FROM {{ ref('class_git_pull_requests_commits') }} AS links FINAL
    INNER JOIN {{ ref('class_git_pull_requests') }} AS prs FINAL
        ON prs.tenant_id = links.tenant_id
        AND prs.source_id = links.source_id
        AND prs.project_key = links.project_key
        AND prs.repo_slug = links.repo_slug
        AND prs.pr_id = links.pr_id
    INNER JOIN repository_default_branches AS defaults
        ON defaults.tenant_id = links.tenant_id
        AND defaults.source_id = links.source_id
        AND defaults.project_key = links.project_key
        AND defaults.repo_slug = links.repo_slug
    WHERE prs.state = 'MERGED'
      AND prs.destination_branch = defaults.branch_name

    UNION ALL

    SELECT
        branch_change.tenant_id AS tenant_id,
        branch_change.source_id AS source_id,
        branch_change.project_key AS project_key,
        branch_change.repo_slug AS repo_slug,
        branch_change.commit_hash AS commit_hash
    FROM {{ ref('class_git_file_changes') }} AS branch_change FINAL
    INNER JOIN landed_content AS landed
        ON landed.tenant_id IS NOT DISTINCT FROM branch_change.tenant_id
        AND landed.source_id IS NOT DISTINCT FROM branch_change.source_id
        AND landed.project_key = branch_change.project_key
        AND landed.repo_slug = branch_change.repo_slug
        AND landed.file_path = branch_change.file_path
        AND landed.content_identity = {{ git_file_content_identity('branch_change.post_image_oid', 'branch_change.pre_image_oid') }}
    WHERE (branch_change.tenant_id, branch_change.source_id, branch_change.project_key, branch_change.repo_slug, branch_change.commit_hash) IN (
        SELECT tenant_id, source_id, project_key, repo_slug, commit_hash
        FROM unlanded_commits
    )
)
