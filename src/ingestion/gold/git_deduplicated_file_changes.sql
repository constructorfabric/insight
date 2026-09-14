{{ config(
    materialized='table',
    engine='MergeTree',
    order_by=['tenant_id', 'data_source', 'commit_hash'],
    schema=var('gold_database'),
    settings={'allow_nullable_key': 1},
    tags=['gold'],
    query_settings=metric_serving_query_settings()
) }}

-- One row per change CONTENT, not per commit that carries it. The same content
-- entering a repository on two lines of history — a branch whose copy of a
-- tree also landed on the default branch, a cherry-pick, a squash that
-- re-applies its branch's whole span, a reverted-then-restored file — is one
-- authored change, and summing every carrier's diff would count those lines
-- more than once. `git_file_content_identity` is what "same content" means.
--
-- Earliest commit wins, so the value lands in the period the content was first
-- authored and does not move when a later commit repeats it.
--
-- The commit_hash tie-breaker keeps rows whose identity is UNKNOWN (a source
-- that reports no oid, or a row collected before the proxy did) distinct per
-- commit: without it every such row for one path would collapse into one,
-- because an unknown identity equals every other unknown identity.
--
-- The superseded set is the case content identity alone cannot reach: a squash
-- whose branch was only PARTLY collected carries the collected commits' work
-- under a span no single commit made, so nothing folds it. See
-- git_superseded_file_changes.
--
-- A model rather than a CTE of the evidence build: the collapse spans every
-- collected file change, and a CTE would run it once per reference inside a
-- query already holding the rest of that build.
WITH collected_file_changes AS (
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
        lines_removed,
        observed_at,
        committer_date,
        {{ git_file_content_identity('post_image_oid', 'pre_image_oid') }} AS content_identity,
        if(
            coalesce(pre_image_oid, '') = ''
                AND coalesce(post_image_oid, '') = '',
            commit_hash,
            ''
        ) AS unknown_content_carrier
    FROM {{ ref('git_commit_file_changes') }}
    WHERE (tenant_id, data_source, commit_hash, file_path) NOT IN (
        SELECT tenant_id, data_source, commit_hash, file_path
        FROM {{ ref('git_superseded_file_changes') }}
    )
),
earliest_carriers AS (
    SELECT
        tenant_id,
        data_source,
        project_key,
        repo_slug,
        file_path,
        -- INVARIANT: committer_date breaks the tie, and must stay ahead of the
        -- hash. observed_at is the AUTHOR date, which a rebase preserves — so
        -- the copy and its original tie there, and without this the survivor
        -- (and with it the repository and branch scope the lines are filed
        -- under) would be decided by comparing hashes. #3153
        --
        -- SAFETY: ONE argMin over a tuple, never one per column. Carriers can
        -- tie on all three ordering fields, and independent argMin calls are
        -- then free to take each column from a different carrier and emit a
        -- row no commit reported.
        argMin(
            (source_id, commit_hash, file_extension, change_type, lines_added, lines_removed),
            (observed_at, committer_date, commit_hash)
        ) AS carrier
    FROM collected_file_changes
    GROUP BY
        tenant_id,
        data_source,
        project_key,
        repo_slug,
        file_path,
        content_identity,
        unknown_content_carrier
)
SELECT
    tenant_id,
    tupleElement(carrier, 1) AS source_id,
    project_key,
    repo_slug,
    tupleElement(carrier, 2) AS commit_hash,
    data_source,
    file_path,
    tupleElement(carrier, 3) AS file_extension,
    tupleElement(carrier, 4) AS change_type,
    tupleElement(carrier, 5) AS lines_added,
    tupleElement(carrier, 6) AS lines_removed
FROM earliest_carriers
