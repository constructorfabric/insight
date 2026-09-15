-- depends_on: {{ ref('bitbucket_cloud__bronze_promoted') }}
{{ config(
    materialized='incremental',
    unique_key='unique_key',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    schema='staging',
    tags=['bitbucket-cloud', 'silver:class_git_pull_requests']
) }}

-- Bitbucket carries no diff totals on the pull request itself, so the per-file
-- diffstat rows are the only source of line counts and are summed here.
--
-- INVARIANT: the row key is the file path, so a ReplacingMergeTree replaces a
-- file's row and never removes one — a file a rebase dropped out of the diff
-- keeps its row for ever. The parent's update stamp is the only thing saying
-- which rows belong to the diff the request has NOW, so the newest stamp's rows
-- are taken WHOLE. Resolving each file to its own newest row keeps the dropped
-- file instead, because nothing newer exists to displace it. #3362
--
-- A row with no usable stamp sorts to the epoch, so a request whose rows all
-- predate the stamp is summed entire, as it was before this rule. #3362
WITH diffstat_rows AS (
    SELECT
        tenant_id,
        source_id,
        repo_full_name,
        pr_id,
        COALESCE(parseDateTimeBestEffortOrNull(pr_updated_on), toDateTime(0)) AS generation,
        lines_added,
        lines_removed,
        _airbyte_extracted_at
    FROM {{ source('bronze_bitbucket_cloud', 'pull_request_diffstat') }} FINAL
),

diffstat_newest_generation AS (
    SELECT
        tenant_id,
        source_id,
        repo_full_name,
        pr_id,
        max(generation) AS generation
    FROM diffstat_rows
    GROUP BY tenant_id, source_id, repo_full_name, pr_id
),

diff_stats AS (
    SELECT
        stat.tenant_id AS tenant_id,
        stat.source_id AS source_id,
        stat.repo_full_name AS repo_full_name,
        stat.pr_id AS pr_id,
        count() AS files_changed,
        sum(stat.lines_added) AS lines_added,
        sum(stat.lines_removed) AS lines_removed,
        max(stat._airbyte_extracted_at) AS _airbyte_extracted_at,
        1 AS matched
    FROM diffstat_rows AS stat
    INNER JOIN diffstat_newest_generation AS newest
        ON newest.tenant_id = stat.tenant_id
        AND newest.source_id = stat.source_id
        AND newest.repo_full_name = stat.repo_full_name
        AND newest.pr_id = stat.pr_id
    WHERE stat.generation = newest.generation
    GROUP BY stat.tenant_id, stat.source_id, stat.repo_full_name, stat.pr_id
),

-- A pull request records no close time of its own; the terminal update event
-- in its activity does.
terminal_activity AS (
    SELECT
        tenant_id,
        source_id,
        repo_full_name,
        pr_id,
        max(event_date) AS closed_on,
        max(_airbyte_extracted_at) AS _airbyte_extracted_at
    FROM {{ source('bronze_bitbucket_cloud', 'pull_request_activity') }} FINAL
    WHERE kind = 'update'
      AND update_state IN ('MERGED', 'DECLINED', 'SUPERSEDED')
    GROUP BY tenant_id, source_id, repo_full_name, pr_id
),

-- Every entry in the activity that is NOT a terminal state change. What it
-- gives is the moment the request last visibly moved for a reason we can name,
-- which is how the projection below tells a bumped `updated_on` that means "the
-- vendor closed this silently" from one that merely means "someone commented".
non_terminal_activity AS (
    SELECT
        tenant_id,
        source_id,
        repo_full_name,
        pr_id,
        max(parseDateTimeBestEffortOrNull(event_date)) AS last_seen_at,
        max(_airbyte_extracted_at) AS _airbyte_extracted_at
    FROM {{ source('bronze_bitbucket_cloud', 'pull_request_activity') }} FINAL
    WHERE NOT (kind = 'update' AND update_state IN ('MERGED', 'DECLINED', 'SUPERSEDED'))
    GROUP BY tenant_id, source_id, repo_full_name, pr_id
),

-- WORKAROUND: a merge reached by pushing a commit that carries the request's
-- head changes the state with no merge ACTION, so the activity holds no
-- terminal entry to date it by — and a merged request with no close time is
-- dropped by every measure that dates by the close. The commit the request
-- names as its merge is the corroboration that the merge happened. #3051
--
-- INVARIANT: Bitbucket answers `merge_commit.hash` with exactly 12 characters
-- where the commit stream carries the full 40, so the join matches on 12 and
-- the projection verifies the whole reported hash against the one that
-- resolved. Same rule as `git_merge_result_match`, which resolves the same
-- hash in gold.
merge_commit_times AS (
    SELECT
        tenant_id,
        source_id,
        project_key,
        repo_slug,
        commit_prefix,
        -- INVARIANT: the HAVING below leaves exactly one distinct hash per
        -- group, so this IS that commit's hash.
        min(commit_hash) AS resolved_hash,
        min(landed_at) AS landed_at,
        1 AS matched,
        max(_airbyte_extracted_at) AS _airbyte_extracted_at
    FROM (
        SELECT
            tenant_id,
            source_id,
            arrayElement(splitByChar('/', COALESCE(repository, '')), -2) AS project_key,
            replaceRegexpOne(arrayElement(splitByChar('/', COALESCE(repository, '')), -1), '\\.git$', '') AS repo_slug,
            substring(COALESCE(sha, ''), 1, 12) AS commit_prefix,
            COALESCE(sha, '') AS commit_hash,
            -- The committer date, unlike `class_git_commits`, which prefers the
            -- author date: a rebase keeps the author date, so it says when the
            -- work was written, and what is wanted here is when this commit
            -- came into being. Author date only as a fallback, for a source
            -- that left the committer date empty.
            COALESCE(
                parseDateTimeBestEffortOrNull(committed_date),
                parseDateTimeBestEffortOrNull(authored_date)
            ) AS landed_at,
            _airbyte_extracted_at
        FROM {{ source('bronze_bitbucket_cloud', 'commits') }} FINAL
        WHERE COALESCE(sha, '') != ''
          -- Bound the scan to hashes some merged request actually names.
          AND substring(COALESCE(sha, ''), 1, 12) IN (
              SELECT substring(COALESCE(merge_commit_sha, ''), 1, 12)
              FROM {{ source('bronze_bitbucket_cloud', 'pull_requests') }} FINAL
              WHERE state = 'MERGED' AND COALESCE(merge_commit_sha, '') != ''
          )
    )
    GROUP BY tenant_id, source_id, project_key, repo_slug, commit_prefix
    -- A prefix is a weak key. Two commits sharing it in one repository leave
    -- nothing to say which merge this was, so such a prefix resolves nothing
    -- rather than guessing.
    HAVING uniqExact(commit_hash) = 1
)

SELECT
    pr.tenant_id AS tenant_id,
    pr.source_id AS source_id,
    pr.unique_key AS unique_key,
    splitByChar('/', COALESCE(pr.repo_full_name, ''))[1] AS project_key,
    splitByChar('/', COALESCE(pr.repo_full_name, ''))[2] AS repo_slug,
    COALESCE(pr.id, 0) AS pr_id,
    COALESCE(pr.id, 0) AS pr_number,
    COALESCE(pr.title, '') AS title,
    COALESCE(pr.description, '') AS description,
    -- A superseded pull request is a declined one to every consumer.
    multiIf(
        pr.state = 'SUPERSEDED', 'DECLINED',
        COALESCE(pr.state, '')
    ) AS state,
    COALESCE(pr.author_display_name, '') AS author_name,
    -- Bitbucket exposes no address on the participant object.
    '' AS author_email,
    -- ADR-0002 account key, normalized the way bitbucket_cloud__identity_inputs
    -- writes it, so the account binding join matches byte-for-byte.
    lower(trimBoth(COALESCE(pr.author_account_id, ''))) AS author_account_id,
    COALESCE(pr.source_branch, '') AS source_branch,
    COALESCE(pr.destination_branch, '') AS destination_branch,
    parseDateTimeBestEffortOrNull(pr.created_on) AS created_on,
    parseDateTimeBestEffortOrNull(pr.updated_on) AS updated_on,
    -- The close time, spelled out in silver/git/README.md. Branch order is the
    -- rule: the terminal activity entry decides whenever one parses; failing
    -- that, a MERGED request the collected commits corroborate takes its own
    -- last update when nothing in its activity accounts for that update, and
    -- the merge commit's timestamp when something does; a request nothing
    -- corroborates gets no close time at all. #3051
    --
    -- INVARIANT: the RECOVERED candidates are taken only from the creation
    -- onwards, and none is invented — where no candidate qualifies the answer
    -- is no close time rather than the creation. The terminal entry above is
    -- taken as the source reports it.
    multiIf(
        COALESCE(pr.state, '') NOT IN ('MERGED', 'DECLINED', 'SUPERSEDED'), CAST(NULL AS Nullable(DateTime)),
        -- The PARSE, not the string: an unparseable event date must fall
        -- through to the recovery rather than answer with nothing. #3153
        parseDateTimeBestEffortOrNull(activity.closed_on) IS NOT NULL,
            parseDateTimeBestEffortOrNull(activity.closed_on),
        COALESCE(pr.state, '') != 'MERGED', CAST(NULL AS Nullable(DateTime)),
        -- Uncorroborated: no commit resolved for the reported hash, or the one
        -- that did does not carry the whole of it. WORKAROUND: the second half
        -- belongs in the join, but ClickHouse refuses a non-equality over both
        -- sides there under `join_use_nulls`.
        COALESCE(merge_commit.matched, 0) != 1
            OR NOT startsWith(
                COALESCE(merge_commit.resolved_hash, ''),
                COALESCE(pr.merge_commit_sha, '')
            ), CAST(NULL AS Nullable(DateTime)),
        COALESCE(seen.last_seen_at IS NULL OR updated_on > seen.last_seen_at, 0)
            AND updated_on IS NOT NULL
            AND COALESCE(updated_on >= created_on, 1), updated_on,
        merge_commit.landed_at IS NOT NULL
            AND COALESCE(merge_commit.landed_at >= created_on, 1), merge_commit.landed_at,
        updated_on IS NOT NULL
            AND COALESCE(updated_on >= created_on, 1), updated_on,
        CAST(NULL AS Nullable(DateTime))
    ) AS closed_on,
    -- The close time as the SOURCE stated it: the terminal entry in the
    -- request's activity, never a recovered candidate. The recovery above
    -- corroborates WHICH DAY a merge landed on, which is what a count needs; a
    -- duration measured to it would report an interval nobody observed, so the
    -- duration measures read this column and drop the request when it is null.
    -- #3362
    if(
        COALESCE(pr.state, '') IN ('MERGED', 'DECLINED', 'SUPERSEDED'),
        parseDateTimeBestEffortOrNull(activity.closed_on),
        CAST(NULL AS Nullable(DateTime))
    ) AS closed_on_reported,
    COALESCE(pr.merge_commit_sha, '') AS merge_commit_hash,
    -- An unmatched join partner and a genuinely empty pull request both read
    -- as 0 through COALESCE; only the marker separates "not collected yet"
    -- from "changed nothing", and the class columns are nullable to say so.
    if(ds.matched = 1, toNullable(toInt64(COALESCE(ds.files_changed, 0))), NULL) AS files_changed,
    if(ds.matched = 1, toNullable(toInt64(COALESCE(ds.lines_added, 0))), NULL) AS lines_added,
    if(ds.matched = 1, toNullable(toInt64(COALESCE(ds.lines_removed, 0))), NULL) AS lines_removed,
    'insight_bitbucket_cloud' AS data_source,
    toUnixTimestamp64Milli(now64()) AS _version,
    -- Diff stats, activity and commits are their own streams: a late arrival
    -- must re-trigger the pull-request row, which is not re-fetched on its own.
    greatest(
        pr._airbyte_extracted_at,
        COALESCE(ds._airbyte_extracted_at, pr._airbyte_extracted_at),
        COALESCE(activity._airbyte_extracted_at, pr._airbyte_extracted_at),
        COALESCE(seen._airbyte_extracted_at, pr._airbyte_extracted_at),
        COALESCE(merge_commit._airbyte_extracted_at, pr._airbyte_extracted_at)
    ) AS _airbyte_extracted_at
FROM {{ source('bronze_bitbucket_cloud', 'pull_requests') }} AS pr FINAL
LEFT JOIN diff_stats AS ds
    ON ds.tenant_id = pr.tenant_id
    AND ds.source_id = pr.source_id
    AND ds.repo_full_name = pr.repo_full_name
    AND ds.pr_id = pr.id
LEFT JOIN terminal_activity AS activity
    ON activity.tenant_id = pr.tenant_id
    AND activity.source_id = pr.source_id
    AND activity.repo_full_name = pr.repo_full_name
    AND activity.pr_id = pr.id
LEFT JOIN non_terminal_activity AS seen
    ON seen.tenant_id = pr.tenant_id
    AND seen.source_id = pr.source_id
    AND seen.repo_full_name = pr.repo_full_name
    AND seen.pr_id = pr.id
-- Scoped to the request's own repository: a 12-character prefix is a weak key,
-- and a commit elsewhere that happens to share it is another repository's work.
LEFT JOIN merge_commit_times AS merge_commit
    ON merge_commit.tenant_id = pr.tenant_id
    AND merge_commit.source_id = pr.source_id
    AND merge_commit.project_key = splitByChar('/', COALESCE(pr.repo_full_name, ''))[1]
    AND merge_commit.repo_slug = splitByChar('/', COALESCE(pr.repo_full_name, ''))[2]
    AND merge_commit.commit_prefix = substring(COALESCE(pr.merge_commit_sha, ''), 1, 12)
{% if is_incremental() %}
WHERE greatest(
    pr._airbyte_extracted_at,
    COALESCE(ds._airbyte_extracted_at, pr._airbyte_extracted_at),
    COALESCE(activity._airbyte_extracted_at, pr._airbyte_extracted_at),
    COALESCE(seen._airbyte_extracted_at, pr._airbyte_extracted_at),
    COALESCE(merge_commit._airbyte_extracted_at, pr._airbyte_extracted_at)
) > (SELECT max(_airbyte_extracted_at) FROM {{ this }})
{% endif %}
