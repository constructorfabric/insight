{{ config(
    materialized='table',
    engine='MergeTree',
    order_by=['tenant_id', 'author_email', 'authored_at'],
    partition_by='toYYYYMM(authored_date)',
    schema=var('gold_database'),
    settings={'allow_nullable_key': 1},
    tags=['gold'],
    query_settings=metric_serving_query_settings()
) }}

-- Authored file changes: one row per change a person wrote — this file, in
-- this commit, adding these lines.
--
-- INVARIANT: a projection of the shared file-change stage, so metric evidence
-- and drilldown read the same derivation.

WITH
-- One row per change CONTENT, not per commit that carries it: the same content
-- entering a repository twice (a squash, a cherry-pick, a restore) is one
-- authored change, and summing both commits' diffs would double its lines.
-- Earliest commit wins, so the value does not move when a later commit repeats.
--
-- INVARIANT: the commit_hash tie-breaker keeps rows with no oid distinct per
-- commit — LIMIT 1 BY reads their NULL keys as equal and would collapse them.
deduplicated_file_changes AS (
    SELECT
        tenant_id,
        source_id,
        project_key,
        repo_slug,
        commit_hash,
        data_source,
        file_path,
        file_extension,
        change_type,
        lines_added,
        lines_removed
    FROM {{ ref('git_commit_file_changes') }}
    ORDER BY observed_at, commit_hash
    LIMIT 1 BY
        tenant_id,
        data_source,
        project_key,
        repo_slug,
        file_path,
        lower(change_type),
        pre_image_oid,
        post_image_oid,
        if(
            coalesce(pre_image_oid, '') = ''
                AND coalesce(post_image_oid, '') = '',
            commit_hash,
            ''
        )
)

SELECT
    file_changes.tenant_id AS tenant_id,
    file_changes.source_id AS source_id,
    file_changes.project_key AS project_key,
    file_changes.repo_slug AS repo_slug,
    file_changes.commit_hash AS commit_hash,
    file_changes.file_path AS file_path,
    commits.entity_id AS author_email,
    commits.author_name AS author_name,
    -- SAFETY: the commit stage admits no row without a date, and neither a
    -- partition key nor a sort key may be nullable.
    assumeNotNull(commits.observed_at) AS authored_at,
    assumeNotNull(commits.metric_date) AS authored_date,
    {{ git_file_category('file_changes.file_path') }} AS category,
    {{ git_file_category_label('category') }} AS category_label,
    if(file_changes.file_extension = '', '__unknown__', lower(file_changes.file_extension)) AS file_extension,
    if(file_changes.file_extension = '', 'Unknown', lower(file_changes.file_extension)) AS file_extension_label,
    if(file_changes.change_type = '', '__unknown__', lower(file_changes.change_type)) AS change_type,
    multiIf(
        file_changes.change_type = '', 'Unknown',
        lower(file_changes.change_type) = 'added', 'Added',
        lower(file_changes.change_type) = 'modified', 'Modified',
        lower(file_changes.change_type) = 'renamed', 'Renamed',
        lower(file_changes.change_type) = 'deleted', 'Deleted',
        file_changes.change_type
    ) AS change_type_label,
    -- INVARIANT: inherited from the commit, never recomputed — lines belong to
    -- the bucket their commit belongs to.
    commits.branch_scope_value AS branch_scope,
    commits.branch_scope_label AS branch_scope_label,
    commits.repository_value AS repository,
    commits.repository_label AS repository_label,
    commits.project_value AS project,
    commits.project_label AS project_label,
    commits.source_value AS source,
    commits.source_label AS source_label,
    file_changes.lines_added AS lines_added,
    file_changes.lines_removed AS lines_removed
FROM deduplicated_file_changes AS file_changes
INNER JOIN {{ ref('git_authored_commits') }} AS commits
    ON commits.tenant_id = file_changes.tenant_id
    AND commits.data_source = file_changes.data_source
    AND commits.commit_hash = file_changes.commit_hash
