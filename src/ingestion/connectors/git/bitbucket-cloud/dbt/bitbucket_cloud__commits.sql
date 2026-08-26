-- depends_on: {{ ref('bitbucket_cloud__bronze_promoted') }}
{{ config(
    materialized='incremental',
    unique_key='unique_key',
    order_by=['unique_key'],
    on_schema_change='append_new_columns',
    settings={'allow_nullable_key': 1},
    schema='staging',
    tags=['bitbucket-cloud', 'silver:class_git_commits']
) }}

-- branch is '' by construction: the proxy walks a repository once rather than
-- once per branch, so a commit carries no branch name, only the membership
-- flag below.
-- INVARIANT: is_in_default_branch is reachability AT SYNC TIME. A commit first
-- seen on a feature branch stays 0 unless a later sync re-reads it, so a merge
-- outside the connector's lookback_window never corrects it.
SELECT
    tenant_id,
    source_id,
    unique_key,
    arrayElement(splitByChar('/', COALESCE(repository, '')), -2) AS project_key,
    replaceRegexpOne(arrayElement(splitByChar('/', COALESCE(repository, '')), -1), '\\.git$', '') AS repo_slug,
    COALESCE(sha, '') AS commit_hash,
    '' AS branch,
    CAST(is_in_default_branch AS Nullable(UInt8)) AS is_default_branch,
    COALESCE(author_name, '') AS author_name,
    COALESCE(author_email, '') AS author_email,
    COALESCE(committer_name, '') AS committer_name,
    COALESCE(committer_email, '') AS committer_email,
    COALESCE(message, '') AS message,
    parseDateTimeBestEffortOrNull(committed_date) AS date,
    toNullable(COALESCE(changed_files, 0)) AS files_changed,
    toNullable(COALESCE(additions, 0)) AS lines_added,
    toNullable(COALESCE(deletions, 0)) AS lines_removed,
    if(COALESCE(is_merge, false), 1, 0) AS is_merge_commit,
    'insight_bitbucket_cloud' AS data_source,
    toUnixTimestamp64Milli(now64()) AS _version,
    _airbyte_extracted_at,
    patch_id
-- FINAL: the lookback window re-reads a commit under the same unique_key, and
-- its membership flag can differ between the two rows. Without FINAL a
-- full-refresh build inserts both into staging under one now64() _version, and
-- union_by_tag's dedup orders by that column — a tie it cannot break. See
-- ADR-0001.
FROM {{ source('bronze_bitbucket_cloud', 'commits') }} FINAL
{% if is_incremental() %}
WHERE _airbyte_extracted_at > (SELECT max(_airbyte_extracted_at) FROM {{ this }})
{% endif %}
