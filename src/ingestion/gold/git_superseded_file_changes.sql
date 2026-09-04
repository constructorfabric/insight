{{ config(
    materialized='table',
    engine='MergeTree',
    order_by=['tenant_id', 'data_source', 'commit_hash'],
    schema=var('gold_database'),
    settings={'allow_nullable_key': 1},
    tags=['gold'],
    query_settings=metric_serving_query_settings()
) }}

-- Paths on a merge-result commit whose work a COLLECTED commit of the same
-- pull request already carries. The content dedup cannot reach them: a squash
-- re-applies its branch as one span, and when only PART of the branch was
-- collected the span matches no collected commit's own transition, so both
-- the original and the squash's copy of it count.
--
-- Only PARTIAL coverage reaches this model. With every linked commit collected
-- the whole squash is already excluded as a derived commit
-- (git_derived_commits), and with none collected it is the only record of the
-- work and every path of it must count.
--
-- Suppressed per PATH, not per commit: the squash is the only record of the
-- commits that were never collected, so its rows for paths no collected commit
-- touched stay. What cannot be recovered is a lost commit's delta on a path a
-- collected commit also touched — that work is unknowable, and undercounting
-- it is the deliberate trade against counting the collected commit's lines
-- twice.
--
-- The join to the file-change class is what restricts the branch side to
-- collected commits: an uncollected commit reaches no file row.

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
-- SAFETY: a prefix naming more than one collected commit suppresses nothing,
-- for the same reason git_derived_commits marks nothing — see
-- `git_merge_result_match`.
resolved_results AS (
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
    WHERE links.collected_branch_commit_count > 0
      AND links.collected_branch_commit_count < length(links.linked_hashes)
    GROUP BY
        prs.tenant_id,
        prs.source_id,
        prs.project_key,
        prs.repo_slug,
        prs.pr_id,
        prs.data_source
    HAVING length(candidates) = 1
),
partly_collected_results AS (
    SELECT DISTINCT
        tenant_id,
        data_source,
        arrayElement(candidates, 1) AS commit_hash,
        linked_hashes
    FROM resolved_results
    -- A fast-forward merge promotes an original, which owns its own rows.
    WHERE NOT has(linked_hashes, arrayElement(candidates, 1))
),
linked_commit_of_result AS (
    SELECT
        tenant_id,
        data_source,
        commit_hash,
        linked_hash
    FROM partly_collected_results
    ARRAY JOIN linked_hashes AS linked_hash
)
SELECT DISTINCT
    result.tenant_id AS tenant_id,
    result.data_source AS data_source,
    result.commit_hash AS commit_hash,
    branch_change.file_path AS file_path
FROM linked_commit_of_result AS result
INNER JOIN {{ ref('class_git_file_changes') }} AS branch_change FINAL
    ON branch_change.tenant_id IS NOT DISTINCT FROM result.tenant_id
    AND branch_change.data_source = result.data_source
    AND branch_change.commit_hash = result.linked_hash
