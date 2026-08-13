-- depends_on: {{ ref('github__bronze_promoted') }}
{{ config(
    materialized='incremental',
    unique_key='unique_key',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    schema='staging',
    tags=['github', 'silver:class_git_commits']
) }}

-- branch is '' by construction: the proxy walks a repository once rather than
-- once per branch, so a commit carries no branch name. Matches gitlab.
SELECT
    tenant_id,
    source_id,
    unique_key,
    arrayElement(splitByChar('/', COALESCE(repository, '')), -2) AS project_key,
    replaceRegexpOne(arrayElement(splitByChar('/', COALESCE(repository, '')), -1), '\\.git$', '') AS repo_slug,
    COALESCE(sha, '') AS commit_hash,
    '' AS branch,
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
    'insight_github' AS data_source,
    toUnixTimestamp64Milli(now64()) AS _version,
    _airbyte_extracted_at
FROM {{ source('bronze_github', 'commits') }}
{% if is_incremental() %}
WHERE _airbyte_extracted_at > (SELECT max(_airbyte_extracted_at) FROM {{ this }})
{% endif %}
