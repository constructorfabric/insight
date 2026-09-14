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
-- collapse below keeps one copy, and every row carries the surviving commit's
-- coordinates.
WITH collapsed_file_changes AS (
    SELECT
        tenant_id,
        data_source,
        commit_hash,
        file_path,
        pre_image_oid,
        post_image_oid,
        -- SAFETY: ONE argMin over a tuple, never one per column. Copies of a
        -- change can tie on the repository coordinates, and independent argMin
        -- calls are then free to take each column from a different copy and
        -- emit a row no repository reported.
        argMin(
            (file_extension, change_type, lines_added, lines_removed),
            (source_id, project_key, repo_slug)
        ) AS carrier
    FROM {{ ref('class_git_file_changes') }} FINAL
    -- INVARIANT: change_type is grouped case-folded and read back raw. Two
    -- spellings of one change are one change, and keeping both would double
    -- the path's lines wherever a reader folds the case itself.
    GROUP BY
        tenant_id,
        data_source,
        commit_hash,
        file_path,
        lower(change_type),
        pre_image_oid,
        post_image_oid
)
SELECT
    commits.tenant_id AS tenant_id,
    commits.source_id AS source_id,
    commits.project_key AS project_key,
    commits.repo_slug AS repo_slug,
    commits.commit_hash AS commit_hash,
    commits.observed_at AS observed_at,
    commits.committer_date AS committer_date,
    raw_file_change.data_source AS data_source,
    raw_file_change.file_path AS file_path,
    tupleElement(raw_file_change.carrier, 1) AS file_extension,
    tupleElement(raw_file_change.carrier, 2) AS change_type,
    tupleElement(raw_file_change.carrier, 3) AS lines_added,
    tupleElement(raw_file_change.carrier, 4) AS lines_removed,
    raw_file_change.pre_image_oid AS pre_image_oid,
    raw_file_change.post_image_oid AS post_image_oid
FROM collapsed_file_changes AS raw_file_change
INNER JOIN {{ ref('git_authored_commits') }} AS commits
    ON commits.tenant_id = raw_file_change.tenant_id
    AND commits.data_source = raw_file_change.data_source
    AND commits.commit_hash = raw_file_change.commit_hash
