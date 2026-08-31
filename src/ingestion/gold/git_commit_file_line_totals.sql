{{ config(
    materialized='table',
    engine='MergeTree',
    order_by=['tenant_id', 'data_source', 'commit_hash'],
    schema=var('gold_database'),
    settings={'allow_nullable_key': 1},
    tags=['gold'],
    query_settings=metric_serving_query_settings()
) }}

-- One row per commit the file-change stream reached, over the changes AS
-- COLLECTED — before the content dedup removes the ones an earlier commit
-- already carries. Materialized so the aggregate runs once per build in its
-- own query budget rather than once per place the evidence build reads it.
--
-- `file_change_rows` is the membership answer, and it is a count rather than a
-- sum for a reason: the line columns are Nullable, so a group of all-NULL rows
-- sums to NULL and could not be told apart from a commit the stream never
-- reached. The count is what says "collected", the sums say how much.
--
-- Keyed on the canonical commit grain, the one git_authored_commits collapses
-- to and git_commit_file_changes already attaches on. Every row there carries
-- the surviving commit's coordinates, so grouping by the repository tuple would
-- produce the same groups through two more Nullable join keys.
SELECT
    tenant_id,
    data_source,
    commit_hash,
    count() AS file_change_rows,
    sum(lines_added) AS lines_added,
    sum(lines_removed) AS lines_removed
FROM {{ ref('git_commit_file_changes') }}
GROUP BY tenant_id, data_source, commit_hash
