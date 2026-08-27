-- depends_on: {{ ref('github__bronze_promoted') }}
{{ config(
    materialized='incremental',
    unique_key='unique_key',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    schema='staging',
    tags=['github', 'silver:class_git_pr_review_events']
) }}

-- One address per account for the class's flat actor_email: a real address
-- outranks GitHub's noreply form, and the earliest-observed pair wins so the
-- pick is stable across syncs.
WITH account_email AS (
    SELECT
        tenant_id,
        source_id,
        account_id,
        argMin(email, (email LIKE '%@users.noreply.github.com', observed_at, email)) AS email
    FROM {{ ref('github__account_emails') }}
    GROUP BY tenant_id, source_id, account_id
),

-- The conversation-comment endpoint is repo-wide and answers for plain issues
-- as well, so only the numbers that name a pull request are kept.
pull_request_numbers AS (
    SELECT
        tenant_id,
        source_id,
        repo_full_name,
        number,
        max(_airbyte_extracted_at) AS pull_request_extracted_at
    FROM {{ source('bronze_github', 'pull_requests') }} FINAL
    GROUP BY tenant_id, source_id, repo_full_name, number
),

events AS (
    SELECT
        r.tenant_id AS tenant_id,
        r.source_id AS source_id,
        r.unique_key AS unique_key,
        splitByChar('/', COALESCE(r.repo_full_name, ''))[1] AS project_key,
        splitByChar('/', COALESCE(r.repo_full_name, ''))[2] AS repo_slug,
        COALESCE(r.pull_number, 0) AS pr_id,
        COALESCE(r.pull_number, 0) AS pr_number,
        'review' AS event_kind,
        lower(COALESCE(r.state, '')) AS review_state,
        COALESCE(r.author_login, '') AS actor_login,
        COALESCE(r.author_login, '') AS actor_name,
        toString(COALESCE(r.author_id, 0)) AS actor_account_id,
        parseDateTimeBestEffortOrNull(r.submitted_at) AS created_at,
        r._airbyte_extracted_at AS _airbyte_extracted_at
    FROM {{ source('bronze_github', 'pull_request_reviews') }} AS r FINAL

    UNION ALL

    SELECT
        c.tenant_id AS tenant_id,
        c.source_id AS source_id,
        c.unique_key AS unique_key,
        splitByChar('/', COALESCE(c.repo_full_name, ''))[1] AS project_key,
        splitByChar('/', COALESCE(c.repo_full_name, ''))[2] AS repo_slug,
        COALESCE(c.issue_number, 0) AS pr_id,
        COALESCE(c.issue_number, 0) AS pr_number,
        'comment' AS event_kind,
        '' AS review_state,
        COALESCE(c.author_login, '') AS actor_login,
        COALESCE(c.author_login, '') AS actor_name,
        toString(COALESCE(c.author_id, 0)) AS actor_account_id,
        parseDateTimeBestEffortOrNull(c.created_at) AS created_at,
        -- A repository the token cannot see is skipped, not failed, so a
        -- comment can land before its pull request does. Watermarking on the
        -- later of the two lets the row through once the pull request arrives;
        -- keyed on the comment alone it would sit below the mark forever.
        greatest(c._airbyte_extracted_at, p.pull_request_extracted_at) AS _airbyte_extracted_at
    FROM {{ source('bronze_github', 'pull_request_comments') }} AS c FINAL
    INNER JOIN pull_request_numbers AS p
        ON p.tenant_id = c.tenant_id
        AND p.source_id = c.source_id
        AND p.repo_full_name = c.repo_full_name
        AND p.number = c.issue_number

    UNION ALL

    SELECT
        rc.tenant_id AS tenant_id,
        rc.source_id AS source_id,
        rc.unique_key AS unique_key,
        splitByChar('/', COALESCE(rc.repo_full_name, ''))[1] AS project_key,
        splitByChar('/', COALESCE(rc.repo_full_name, ''))[2] AS repo_slug,
        COALESCE(rc.pull_number, 0) AS pr_id,
        COALESCE(rc.pull_number, 0) AS pr_number,
        'comment' AS event_kind,
        '' AS review_state,
        COALESCE(rc.author_login, '') AS actor_login,
        COALESCE(rc.author_login, '') AS actor_name,
        toString(COALESCE(rc.author_id, 0)) AS actor_account_id,
        parseDateTimeBestEffortOrNull(rc.created_at) AS created_at,
        rc._airbyte_extracted_at AS _airbyte_extracted_at
    FROM {{ source('bronze_github', 'pull_request_review_comments') }} AS rc FINAL
)

SELECT
    e.tenant_id AS tenant_id,
    e.source_id AS source_id,
    e.unique_key AS unique_key,
    e.project_key AS project_key,
    e.repo_slug AS repo_slug,
    e.pr_id AS pr_id,
    e.pr_number AS pr_number,
    e.event_kind AS event_kind,
    e.review_state AS review_state,
    e.actor_login AS actor_login,
    e.actor_name AS actor_name,
    e.actor_account_id AS actor_account_id,
    COALESCE(ae.email, '') AS actor_email,
    e.created_at AS created_at,
    'insight_github' AS data_source,
    toUnixTimestamp64Milli(now64()) AS _version,
    e._airbyte_extracted_at AS _airbyte_extracted_at
FROM events AS e
-- The pairs table carries no extraction time, so a pair learned later does not
-- re-trigger rows already staged; the address catches up when the event's own
-- bronze row is re-extracted.
LEFT JOIN account_email AS ae
    ON ae.tenant_id = e.tenant_id
    AND ae.source_id = e.source_id
    AND ae.account_id = e.actor_account_id
{% if is_incremental() %}
WHERE e._airbyte_extracted_at > (SELECT max(_airbyte_extracted_at) FROM {{ this }})
{% endif %}
