{{ config(
    materialized='table',
    engine='MergeTree',
    order_by=['tenant_id', 'data_source', 'commit_hash'],
    schema=var('gold_database'),
    settings={'allow_nullable_key': 1},
    tags=['gold'],
    query_settings=metric_serving_query_settings()
) }}

-- File changes attached to the commit row that survived the hash collapse.
-- The attach key carries NO repository: commit rows collapse across
-- repositories (a fork and its upstream hold the same hash), and a
-- repo-qualified attach would leave the rows recorded under the repository
-- that lost the collapse matching no commit and contributing nothing. One
-- hash is one diff, so any repository's rows describe the commit — the
-- LIMIT 1 BY collapses the per-repository copies, and every row carries the
-- surviving commit's coordinates.
SELECT
    commits.tenant_id AS tenant_id,
    commits.source_id AS source_id,
    commits.project_key AS project_key,
    commits.repo_slug AS repo_slug,
    commits.commit_hash AS commit_hash,
    commits.observed_at AS observed_at,
    raw_file_change.data_source AS data_source,
    raw_file_change.file_path AS file_path,
    raw_file_change.file_extension AS file_extension,
    raw_file_change.change_type AS change_type,
    raw_file_change.lines_added AS lines_added,
    raw_file_change.lines_removed AS lines_removed,
    raw_file_change.pre_image_oid AS pre_image_oid,
    raw_file_change.post_image_oid AS post_image_oid
FROM {{ ref('class_git_file_changes') }} AS raw_file_change FINAL
INNER JOIN {{ ref('git_authored_commits') }} AS commits
    ON commits.tenant_id = raw_file_change.tenant_id
    AND commits.data_source = raw_file_change.data_source
    AND commits.commit_hash = raw_file_change.commit_hash
ORDER BY raw_file_change.source_id, raw_file_change.project_key, raw_file_change.repo_slug
LIMIT 1 BY
    tenant_id,
    data_source,
    commit_hash,
    file_path,
    lower(change_type),
    pre_image_oid,
    post_image_oid
