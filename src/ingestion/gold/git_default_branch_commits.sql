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
)

SELECT DISTINCT
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
