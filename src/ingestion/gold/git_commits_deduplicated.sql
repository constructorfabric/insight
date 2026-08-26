{{ config(
    materialized='ephemeral',
    tags=['gold']
) }}

-- One row per commit per tenant and source system, with the dimension values
-- and labels every git dataset presents it by. The same commit mirrored into
-- several repositories or collected by two connector instances is one authored
-- commit, and counting each copy would multiply a person's work by how many
-- places it was found.
--
-- Ephemeral: `git_commits` and `git_file_changes` both build on it, and both
-- must settle "which commits are one commit" the same way. Inlining it into
-- each is how the two would drift.
--
-- Identity stays as the source gave it. The person a row belongs to is
-- resolved when a query runs, so an identity correction shows up on the next
-- read instead of waiting for a rebuild.

SELECT
    tenant_id,
    source_id,
    project_key,
    repo_slug,
    commit_hash,
    data_source,
    author_name,
    message,
    lower(trimBoth(author_email)) AS author_email,
    -- assumeNotNull is safe under the WHERE below, and keeps the event time
    -- and the partition key out of Nullable: a partition key may not be
    -- nullable, and a sort key that is costs a read for nothing.
    assumeNotNull(date) AS authored_at,
    toDate(assumeNotNull(date)) AS authored_date,
    lines_added,
    lines_removed,
    -- Reachability at sync time is the wrong tense for "did this work land": a
    -- commit first seen on a feature branch keeps its 0 until a re-read. A
    -- merged request whose destination is the default branch is standing proof
    -- the commit landed, so membership decides alongside the flag.
    if(
        is_default_branch = 1
            OR (tenant_id, source_id, project_key, repo_slug, commit_hash) IN (
                SELECT tenant_id, source_id, project_key, repo_slug, commit_hash
                FROM {{ ref('git_default_branch_commits') }}
            ),
        'default',
        'non_default'
    ) AS branch_scope,
    {{ git_branch_scope_label('branch_scope') }} AS branch_scope_label,
    concat(coalesce(toString(source_id), ''), ':', coalesce(project_key, ''), '/', coalesce(repo_slug, '')) AS repository,
    if(coalesce(project_key, '') = '', coalesce(repo_slug, ''), concat(project_key, '/', repo_slug)) AS repository_label,
    if(coalesce(project_key, '') = '', '__unknown__', concat(coalesce(toString(source_id), ''), ':', project_key)) AS project,
    if(coalesce(project_key, '') = '', 'Unknown', project_key) AS project_label,
    replaceOne(data_source, 'insight_', '') AS source,
    {{ git_source_label('source') }} AS source_label
FROM {{ ref('class_git_commits') }} FINAL
WHERE trimBoth(author_email) != ''
  AND date IS NOT NULL
  AND is_merge_commit = 0
ORDER BY tenant_id, data_source, commit_hash, source_id, project_key, repo_slug
LIMIT 1 BY tenant_id, data_source, commit_hash
