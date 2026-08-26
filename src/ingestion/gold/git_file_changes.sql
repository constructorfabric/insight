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

-- Authored file changes: one row per change a person wrote, at the grain a
-- person recognises — this file, in this commit, adding these lines. Measures
-- sum it by category, extension or change type; drilldown shows the files
-- themselves rather than a daily total that cannot be checked.
--
-- Ordered by tenant, person and time because that is how it is read: one
-- person's changes over a period, never a scan of every change ever made.

WITH
-- One row per change CONTENT, not per commit that carries it. The same content
-- entering a repository on two lines of history — a branch squashed onto the
-- default branch as well, a cherry-pick, a reverted-then-restored file — is one
-- authored change with one oid pair, and summing both commits' diffs would
-- count those lines twice.
--
-- Earliest commit wins, so the value lands in the period the content was first
-- authored and does not move when a later commit repeats it.
--
-- The commit_hash tie-breaker keeps rows whose content identity is unknown (a
-- source that reports no oid) distinct per commit: without it every such row
-- for one path would collapse into one, because LIMIT 1 BY reads their NULL
-- keys as equal.
deduplicated_file_changes AS (
    SELECT
        raw_file_change.tenant_id AS tenant_id,
        raw_file_change.source_id AS source_id,
        raw_file_change.project_key AS project_key,
        raw_file_change.repo_slug AS repo_slug,
        raw_file_change.commit_hash AS commit_hash,
        raw_file_change.data_source AS data_source,
        raw_file_change.file_path AS file_path,
        raw_file_change.file_extension AS file_extension,
        raw_file_change.change_type AS change_type,
        raw_file_change.lines_added AS lines_added,
        raw_file_change.lines_removed AS lines_removed
    FROM {{ ref('class_git_file_changes') }} AS raw_file_change FINAL
    INNER JOIN {{ ref('git_commits_deduplicated') }} AS commits
        ON commits.tenant_id = raw_file_change.tenant_id
        AND commits.source_id = raw_file_change.source_id
        AND commits.project_key = raw_file_change.project_key
        AND commits.repo_slug = raw_file_change.repo_slug
        AND commits.commit_hash = raw_file_change.commit_hash
    ORDER BY commits.authored_at, raw_file_change.commit_hash
    LIMIT 1 BY
        raw_file_change.tenant_id,
        raw_file_change.data_source,
        raw_file_change.project_key,
        raw_file_change.repo_slug,
        raw_file_change.file_path,
        lower(raw_file_change.change_type),
        raw_file_change.pre_image_oid,
        raw_file_change.post_image_oid,
        if(
            coalesce(raw_file_change.pre_image_oid, '') = ''
                AND coalesce(raw_file_change.post_image_oid, '') = '',
            raw_file_change.commit_hash,
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
    commits.author_email AS author_email,
    commits.author_name AS author_name,
    commits.authored_at AS authored_at,
    commits.authored_date AS authored_date,
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
    -- Inherited, not recomputed: lines belong to the bucket their commit
    -- belongs to. A commit in `default` whose lines read `non_default` is the
    -- column disagreement this inheritance exists to prevent.
    commits.branch_scope AS branch_scope,
    commits.branch_scope_label AS branch_scope_label,
    commits.repository AS repository,
    commits.repository_label AS repository_label,
    commits.project AS project,
    commits.project_label AS project_label,
    commits.source AS source,
    commits.source_label AS source_label,
    file_changes.lines_added AS lines_added,
    file_changes.lines_removed AS lines_removed
FROM deduplicated_file_changes AS file_changes
INNER JOIN {{ ref('git_commits_deduplicated') }} AS commits
    ON commits.tenant_id = file_changes.tenant_id
    AND commits.source_id = file_changes.source_id
    AND commits.project_key = file_changes.project_key
    AND commits.repo_slug = file_changes.repo_slug
    AND commits.commit_hash = file_changes.commit_hash
