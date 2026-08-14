-- depends_on: {{ ref('github__bronze_promoted') }}
{{ config(
    materialized='incremental',
    unique_key='unique_key',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    schema='staging',
    tags=['github', 'silver:class_git_pull_requests_comments']
) }}

-- The conversation-comment endpoint is repo-wide and answers for plain issues
-- as well, so only the numbers that name a pull request are kept.
WITH pull_request_numbers AS (
    SELECT
        tenant_id,
        source_id,
        repo_full_name,
        number,
        max(_airbyte_extracted_at) AS pull_request_extracted_at
    FROM {{ source('bronze_github', 'pull_requests') }} FINAL
    GROUP BY tenant_id, source_id, repo_full_name, number
)
SELECT
    c.tenant_id AS tenant_id,
    c.source_id AS source_id,
    c.unique_key AS unique_key,
    splitByChar('/', COALESCE(c.repo_full_name, ''))[1] AS project_key,
    splitByChar('/', COALESCE(c.repo_full_name, ''))[2] AS repo_slug,
    COALESCE(c.issue_number, 0) AS pr_id,
    COALESCE(c.id, 0) AS comment_id,
    COALESCE(c.body, '') AS content,
    COALESCE(c.author_login, '') AS author_name,
    toString(COALESCE(c.author_id, 0)) AS author_uuid,
    parseDateTimeBestEffortOrNull(c.created_at) AS created_at,
    parseDateTimeBestEffortOrNull(c.updated_at) AS updated_at,
    0 AS is_inline,
    '' AS file_path,
    0 AS line_number,
    'insight_github' AS data_source,
    toUnixTimestamp64Milli(now64()) AS _version,
    -- A repository the token cannot see is skipped, not failed, so a comment
    -- can land before its pull request does. Watermarking on the later of the
    -- two lets the row through once the pull request arrives; keyed on the
    -- comment alone it would sit below the mark forever.
    greatest(c._airbyte_extracted_at, p.pull_request_extracted_at) AS _airbyte_extracted_at
FROM {{ source('bronze_github', 'pull_request_comments') }} AS c FINAL
INNER JOIN pull_request_numbers AS p
    ON p.tenant_id = c.tenant_id
    AND p.source_id = c.source_id
    AND p.repo_full_name = c.repo_full_name
    AND p.number = c.issue_number
{% if is_incremental() %}
WHERE greatest(c._airbyte_extracted_at, p.pull_request_extracted_at)
    > (SELECT max(_airbyte_extracted_at) FROM {{ this }})
{% endif %}

UNION ALL

SELECT
    tenant_id,
    source_id,
    unique_key,
    splitByChar('/', COALESCE(repo_full_name, ''))[1] AS project_key,
    splitByChar('/', COALESCE(repo_full_name, ''))[2] AS repo_slug,
    COALESCE(pull_number, 0) AS pr_id,
    COALESCE(id, 0) AS comment_id,
    COALESCE(body, '') AS content,
    COALESCE(author_login, '') AS author_name,
    toString(COALESCE(author_id, 0)) AS author_uuid,
    parseDateTimeBestEffortOrNull(created_at) AS created_at,
    parseDateTimeBestEffortOrNull(updated_at) AS updated_at,
    1 AS is_inline,
    COALESCE(path, '') AS file_path,
    COALESCE(line, 0) AS line_number,
    'insight_github' AS data_source,
    toUnixTimestamp64Milli(now64()) AS _version,
    _airbyte_extracted_at
FROM {{ source('bronze_github', 'pull_request_review_comments') }} FINAL
{% if is_incremental() %}
WHERE _airbyte_extracted_at > (SELECT max(_airbyte_extracted_at) FROM {{ this }})
{% endif %}
