-- depends_on: {{ ref('gitlab__bronze_promoted') }}
{{ config(
    materialized='incremental',
    unique_key='unique_key',
    order_by=['unique_key'],
    on_schema_change='append_new_columns',
    settings={'allow_nullable_key': 1},
    schema='staging',
    tags=['gitlab', 'silver:class_git_commits']
) }}

-- branch is '' by construction: the proxy walks a project once rather than
-- once per branch, so a commit carries no branch name, only the membership
-- flag below.
-- INVARIANT: is_in_default_branch is reachability AT SYNC TIME. A commit first
-- seen on a feature branch stays 0 unless a later sync re-reads it, so a merge
-- outside the connector's lookback_window never corrects it.
SELECT
    tenant_id,
    source_id,
    unique_key,
    arrayStringConcat(arrayPopBack(splitByChar('/', COALESCE(repo_path, ''))), '/') AS project_key,
    arrayElement(splitByChar('/', COALESCE(repo_path, '')), -1) AS repo_slug,
    COALESCE(sha, '') AS commit_hash,
    '' AS branch,
    CAST(is_in_default_branch AS Nullable(UInt8)) AS is_default_branch,
    COALESCE(author_name, '') AS author_name,
    COALESCE(author_email, '') AS author_email,
    COALESCE(committer_name, '') AS committer_name,
    COALESCE(committer_email, '') AS committer_email,
    COALESCE(message, '') AS message,
    -- INVARIANT: the AUTHOR date, with the committer date as a fallback, and
    -- the two PARSES coalesced rather than the two strings — an unparseable
    -- author date must fall through, not win and yield NULL. committed_date is
    -- the stream's cursor and always present.
    coalesce(
        parseDateTimeBestEffortOrNull(authored_date),
        parseDateTimeBestEffortOrNull(committed_date)
    ) AS date,
    toNullable(COALESCE(changed_files, 0)) AS files_changed,
    toNullable(COALESCE(additions, 0)) AS lines_added,
    toNullable(COALESCE(deletions, 0)) AS lines_removed,
    if(COALESCE(is_merge, false), 1, 0) AS is_merge_commit,
    'insight_gitlab' AS data_source,
    toUnixTimestamp64Milli(now64()) AS _version,
    _airbyte_extracted_at,
    patch_id,
    parseDateTimeBestEffortOrNull(committed_date) AS committer_date
-- FINAL: the lookback window re-reads a commit under the same unique_key, and
-- its membership flag can differ between the two rows; union_by_tag's dedup
-- cannot break a tie between two rows staged under one _version.
FROM {{ source('bronze_gitlab', 'commits') }} FINAL
{% if is_incremental() %}
WHERE _airbyte_extracted_at > (SELECT max(_airbyte_extracted_at) FROM {{ this }})
{% endif %}
