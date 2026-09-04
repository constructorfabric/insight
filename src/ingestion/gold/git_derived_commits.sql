{{ config(
    materialized='table',
    engine='MergeTree',
    order_by=['tenant_id', 'data_source', 'commit_hash'],
    schema=var('gold_database'),
    settings={'allow_nullable_key': 1},
    tags=['gold'],
    query_settings=metric_serving_query_settings()
) }}

-- Commits whose work is already counted on another commit. The evidence build
-- excludes them from both the commit and the line figures, so the two
-- describe the same set of authored changes and one change counts once.
--
-- Two reasons, one grain — (tenant_id, data_source, commit_hash, reason):
--
-- * merge_result: the commit a merged pull request produced on its
--   destination — a squash result, or the last rebased copy. Its content is
--   the request's branch commits, which carry the credit. Marked only when
--   EVERY linked commit was collected: with partial coverage the result
--   commit is the only complete record of the work, so it stays counted
--   (the overlap with the collected originals over-counts until coverage
--   completes, which never loses work or per-author credit — the content
--   dedup on file oids still folds the overlapping lines). The result hash
--   is never marked when it is itself a linked commit: a fast-forward merge
--   promotes an original, which stays authored.
--
-- * patch_duplicate: a commit whose diff content (patch id) an earlier commit
--   IN THE SAME REPOSITORY already carries — a rebase copy or a cherry-pick.
--   Scoped to the repository because a patch id identifies likely-duplicate
--   content, not lineage: the same small diff applied independently to two
--   unrelated repositories is two authored changes. The earliest commit
--   wins, so the work lands in the period it was first authored; a
--   merge-result commit never wins, so a same-day squash cannot displace its
--   original. Commits without a patch id (a source that reports none, or a
--   row collected before the source did) have unknown identity and are never
--   collapsed.
--
--   INVARIANT: the committer date ranks after the author date and before the
--   hash. A rebase and a cherry-pick both PRESERVE the author date, so the
--   copies this rule exists to rank tie on `first_seen_date` alone; without a
--   second real key the original would be chosen by comparing hashes. #3153
--
-- Merge commits themselves (two parents) never enter the evidence commit set,
-- so they need no row here.

WITH
collected_branch_commits AS (
    SELECT DISTINCT
        tenant_id,
        source_id,
        project_key,
        repo_slug,
        data_source,
        commit_hash
    FROM {{ ref('class_git_commits') }} FINAL
    WHERE is_merge_commit = 0
),
pull_request_links AS (
    SELECT
        tenant_id,
        source_id,
        project_key,
        repo_slug,
        pr_id,
        groupUniqArray(commit_hash) AS linked_hashes,
        uniqExactIf(
            commit_hash,
            (tenant_id, source_id, project_key, repo_slug, commit_hash) IN (
                SELECT tenant_id, source_id, project_key, repo_slug, commit_hash
                FROM collected_branch_commits
            )
        ) AS collected_branch_commit_count
    FROM {{ ref('class_git_pull_requests_commits') }} FINAL
    GROUP BY tenant_id, source_id, project_key, repo_slug, pr_id
),
-- The requests that produced a result commit at all. Narrowed before the
-- resolution below, whose prefix test is a residual predicate: the join's only
-- equi key is the repository, so every row that reaches it is compared against
-- that repository's whole commit set.
merged_requests AS (
    SELECT
        tenant_id,
        source_id,
        project_key,
        repo_slug,
        data_source,
        pr_id,
        merge_commit_hash
    FROM {{ ref('class_git_pull_requests') }} FINAL
    WHERE state = 'MERGED'
      AND merge_commit_hash != ''
),
-- The result commit each merged request produced, resolved to a collected
-- commit rather than compared to one — see `git_merge_result_match`.
--
-- SAFETY: marking a commit removes its lines along with itself, so a prefix
-- that names more than one collected commit marks neither. Over-counting the
-- request is recoverable; deleting an unrelated author's work is not.
resolved_merge_results AS (
    SELECT
        prs.tenant_id AS tenant_id,
        prs.data_source AS data_source,
        groupUniqArray(result.commit_hash) AS candidates,
        any(links.linked_hashes) AS linked_hashes
    FROM merged_requests AS prs
    INNER JOIN pull_request_links AS links
        ON links.tenant_id = prs.tenant_id
        AND links.source_id = prs.source_id
        AND links.project_key = prs.project_key
        AND links.repo_slug = prs.repo_slug
        AND links.pr_id = prs.pr_id
    INNER JOIN collected_branch_commits AS result
        ON {{ git_merge_result_match('prs', 'result') }}
    WHERE links.collected_branch_commit_count = length(links.linked_hashes)
    GROUP BY
        prs.tenant_id,
        prs.source_id,
        prs.project_key,
        prs.repo_slug,
        prs.pr_id,
        prs.data_source
    HAVING length(candidates) = 1
),
merge_results AS (
    SELECT DISTINCT
        tenant_id,
        data_source,
        arrayElement(candidates, 1) AS commit_hash
    FROM resolved_merge_results
    -- A fast-forward merge promotes an original, which stays authored.
    WHERE NOT has(linked_hashes, arrayElement(candidates, 1))
),
-- One row per commit per repository: patch ids rank within a repository, so a
-- duplicate patch in an unrelated repository is untouched. A hash present in
-- two connected repositories ranks once per repository; DISTINCT below folds
-- the outcome back to the hash grain the anti-join reads.
commit_patches AS (
    SELECT
        tenant_id,
        data_source,
        project_key,
        repo_slug,
        commit_hash,
        min(patch_id) AS commit_patch_id,
        min(date) AS first_seen_date,
        min(committer_date) AS first_carried_date
    FROM {{ ref('class_git_commits') }} FINAL
    WHERE is_merge_commit = 0
      AND coalesce(patch_id, '') != ''
      AND date IS NOT NULL
    GROUP BY tenant_id, data_source, project_key, repo_slug, commit_hash
),
patch_duplicates AS (
    SELECT DISTINCT
        tenant_id,
        data_source,
        commit_hash
    FROM (
        SELECT
            tenant_id,
            data_source,
            commit_hash,
            row_number() OVER (
                PARTITION BY tenant_id, data_source, project_key, repo_slug, commit_patch_id
                ORDER BY is_merge_result, first_seen_date, first_carried_date, commit_hash
            ) AS authored_rank
        FROM (
            SELECT
                tenant_id,
                data_source,
                project_key,
                repo_slug,
                commit_hash,
                commit_patch_id,
                first_seen_date,
                first_carried_date,
                if(
                    (tenant_id, data_source, commit_hash) IN (
                        SELECT tenant_id, data_source, commit_hash
                        FROM merge_results
                    ),
                    1,
                    0
                ) AS is_merge_result
            FROM commit_patches
        )
    )
    WHERE authored_rank > 1
)

SELECT
    tenant_id,
    data_source,
    commit_hash,
    'merge_result' AS reason
FROM merge_results

UNION ALL

SELECT
    tenant_id,
    data_source,
    commit_hash,
    'patch_duplicate' AS reason
FROM patch_duplicates
