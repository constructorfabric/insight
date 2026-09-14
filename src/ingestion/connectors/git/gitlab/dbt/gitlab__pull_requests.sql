-- depends_on: {{ ref('gitlab__bronze_promoted') }}
{{ config(
    materialized='incremental',
    unique_key='unique_key',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    schema='staging',
    tags=['gitlab', 'silver:class_git_pull_requests']
) }}

-- GitLab merge request -> class_git_pull_requests. pr_id and pr_number are
-- both the per-project iid: every child stream is keyed on it. The project
-- roster is an INNER JOIN — a group listing returns merge requests from every
-- project in the group, including forks and excluded paths the roster never
-- admitted, and those must not reach the class.
--
-- Line totals come from the GraphQL diff-stats stream; REST carries only a
-- capped `changes_count` string. The class columns stay NULL until the stats
-- row exists and GitLab has computed it, so "not collected yet" never reads as
-- "changed nothing".
WITH projects AS (
    SELECT
        tenant_id,
        source_id,
        id AS project_id,
        COALESCE(namespace_full_path, '') AS project_key,
        COALESCE(path, '') AS repo_slug,
        _airbyte_extracted_at
    FROM {{ source('bronze_gitlab', 'repositories') }} FINAL
),

diff_stats AS (
    SELECT
        tenant_id,
        source_id,
        project_id,
        mr_iid,
        additions,
        deletions,
        files_changed,
        _airbyte_extracted_at
    FROM {{ source('bronze_gitlab', 'pull_request_diff_stats') }} FINAL
),

-- One address per account for the class's flat author_email: a real address
-- outranks the noreply form, and the earliest-observed pair wins so the pick
-- is stable across syncs. The pairs table carries no extraction time, so an
-- address learned later reaches the row when the merge request is next
-- re-read; the account id below is what attribution rests on.
account_email AS (
    SELECT
        tenant_id,
        source_id,
        account_id,
        argMin(email, (email LIKE '%@users.noreply.%', observed_at, email)) AS email
    FROM {{ ref('gitlab__account_emails') }}
    GROUP BY tenant_id, source_id, account_id
)

SELECT
    mr.tenant_id AS tenant_id,
    mr.source_id AS source_id,
    mr.unique_key AS unique_key,
    p.project_key AS project_key,
    p.repo_slug AS repo_slug,
    COALESCE(mr.iid, 0) AS pr_id,
    COALESCE(mr.iid, 0) AS pr_number,
    COALESCE(mr.title, '') AS title,
    COALESCE(mr.description, '') AS description,
    multiIf(
        mr.state = 'opened', 'OPEN',
        mr.state = 'closed', 'CLOSED',
        mr.state = 'merged', 'MERGED',
        mr.state = 'locked', 'LOCKED',
        upper(COALESCE(mr.state, ''))
    ) AS state,
    COALESCE(mr.author_username, '') AS author_name,
    COALESCE(ae.email, '') AS author_email,
    -- The numeric GitLab user id, stringified — the key gitlab__identity_inputs
    -- binds accounts on. '' when GitLab reports no author (a deleted account).
    if(COALESCE(mr.author_id, 0) > 0, toString(COALESCE(mr.author_id, 0)), '') AS author_account_id,
    COALESCE(mr.source_branch, '') AS source_branch,
    COALESCE(mr.target_branch, '') AS destination_branch,
    parseDateTimeBestEffortOrNull(mr.created_at) AS created_on,
    parseDateTimeBestEffortOrNull(mr.updated_at) AS updated_on,
    parseDateTimeBestEffortOrNull(COALESCE(NULLIF(mr.merged_at, ''), mr.closed_at)) AS closed_on,
    -- A squash merge lands as squash_commit_sha and leaves merge_commit_sha
    -- empty; either is the commit the target branch received.
    COALESCE(NULLIF(mr.merge_commit_sha, ''), mr.squash_commit_sha, '') AS merge_commit_hash,
    -- NULL until GitLab has computed the stats; a pending summary is not a
    -- zero-line change.
    toNullable(toInt64(ds.files_changed)) AS files_changed,
    toNullable(toInt64(ds.additions)) AS lines_added,
    toNullable(toInt64(ds.deletions)) AS lines_removed,
    'insight_gitlab' AS data_source,
    toUnixTimestamp64Milli(now64()) AS _version,
    -- Diff stats are their own stream: a late arrival must re-trigger the
    -- merge request row, which is not re-fetched on its own.
    greatest(
        mr._airbyte_extracted_at,
        COALESCE(ds._airbyte_extracted_at, mr._airbyte_extracted_at)
    ) AS _airbyte_extracted_at
-- FINAL: a merge request updated twice inside one sync's lookback leaves two
-- bronze rows under one key until the parts merge; the class dedup cannot
-- break a tie between them.
FROM {{ source('bronze_gitlab', 'pull_requests') }} AS mr FINAL
INNER JOIN projects AS p
    ON p.tenant_id = mr.tenant_id
    AND p.source_id = mr.source_id
    AND p.project_id = mr.project_id
LEFT JOIN diff_stats AS ds
    ON ds.tenant_id = mr.tenant_id
    AND ds.source_id = mr.source_id
    AND ds.project_id = mr.project_id
    AND ds.mr_iid = mr.iid
LEFT JOIN account_email AS ae
    ON ae.tenant_id = mr.tenant_id
    AND ae.source_id = mr.source_id
    AND ae.account_id = toString(COALESCE(mr.author_id, 0))
{% if is_incremental() %}
-- INVARIANT: the roster row is part of the watermark, so a merge request read
-- before its project reached the roster is picked up once the project does.
WHERE greatest(
    mr._airbyte_extracted_at,
    COALESCE(ds._airbyte_extracted_at, mr._airbyte_extracted_at),
    p._airbyte_extracted_at
) > (SELECT max(_airbyte_extracted_at) FROM {{ this }})
{% endif %}
