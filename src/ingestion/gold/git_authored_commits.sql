{{ config(
    materialized='table',
    engine='MergeTree',
    order_by=['tenant_id', 'data_source', 'commit_hash'],
    schema=var('gold_database'),
    settings={'allow_nullable_key': 1},
    tags=['gold'],
    query_settings=metric_serving_query_settings()
) }}

-- One row per authored commit, with the dimensions every git commit measure
-- carries. Materialized so the FINAL read of the commit class and the hash
-- collapse run once per build in their own query budget, and so the file
-- change models attach to the surviving commit row without repeating it.

SELECT
    tenant_id,
    source_id,
    project_key,
    repo_slug,
    commit_hash,
    data_source,
    author_name,
    message,
    date AS observed_at,
    lower(trimBoth(author_email)) AS entity_id,
    toDate(date) AS metric_date,
    lines_added,
    lines_removed,
    -- A semi-join by tuple rather than a LEFT JOIN: the answer is
    -- membership, and a nullable joined column would need guarding under
    -- either join_use_nulls setting.
    if(
        is_default_branch = 1
            OR (tenant_id, source_id, project_key, repo_slug, commit_hash) IN (
                SELECT tenant_id, source_id, project_key, repo_slug, commit_hash
                FROM {{ ref('git_default_branch_commits') }}
            ),
        'default',
        'non_default'
    ) AS branch_scope_value,
    {{ git_branch_scope_label('branch_scope_value') }} AS branch_scope_label,
    if(coalesce(project_key, '') = '', '__unknown__', concat(coalesce(toString(source_id), ''), ':', project_key)) AS project_value,
    if(coalesce(project_key, '') = '', 'Unknown', project_key) AS project_label,
    concat(coalesce(toString(source_id), ''), ':', coalesce(project_key, ''), '/', coalesce(repo_slug, '')) AS repository_value,
    if(coalesce(project_key, '') = '', coalesce(repo_slug, ''), concat(project_key, '/', repo_slug)) AS repository_label,
    replaceOne(data_source, 'insight_', '') AS source_value,
    {{ git_source_label('source_value') }} AS source_label,
    CAST(
        [
            tuple('branch_scope', branch_scope_value, branch_scope_label),
            tuple('repository', repository_value, repository_label),
            tuple('project', project_value, project_label),
            tuple('source_id', coalesce(toString(source_id), ''), coalesce(toString(source_id), '')),
            tuple('source', source_value, source_label)
        ]
        AS Array(Tuple(key String, value String, label Nullable(String)))
    ) AS source_dimensions
FROM {{ ref('class_git_commits') }} FINAL
WHERE trimBoth(author_email) != ''
  AND date IS NOT NULL
  AND is_merge_commit = 0
  -- A semi-join by tuple for the same reason as branch scope above: a
  -- derived commit (a squash/rebase result of a merged request, or a
  -- later carrier of a patch an earlier commit authored) re-applies work
  -- counted on another commit, so it contributes neither a commit nor —
  -- because every file-change CTE attaches through this set — any lines.
  AND (tenant_id, data_source, commit_hash) NOT IN (
      SELECT tenant_id, data_source, commit_hash
      FROM {{ ref('git_derived_commits') }}
  )
ORDER BY tenant_id, data_source, commit_hash, source_id, project_key, repo_slug
LIMIT 1 BY tenant_id, data_source, commit_hash
