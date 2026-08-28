-- depends_on: {{ ref('bitbucket_cloud__bronze_promoted') }}
{{ config(
    materialized='incremental',
    unique_key='unique_key',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    schema='staging',
    tags=['bitbucket-cloud', 'silver:class_git_pr_review_events']
) }}

-- One address per account for the class's flat actor_email: the
-- earliest-observed pair wins so the pick is stable across syncs.
WITH account_email AS (
    SELECT
        tenant_id,
        source_id,
        account_id,
        argMin(email, (observed_at, email)) AS email
    FROM {{ ref('bitbucket_cloud__account_emails') }}
    GROUP BY tenant_id, source_id, account_id
),

events AS (
    -- Review verdicts live in the activity stream. Its update kind is
    -- pull-request lifecycle, not review activity, and its comment kind
    -- duplicates the comments stream without the comment's own id — both
    -- stay out.
    SELECT
        a.tenant_id AS tenant_id,
        a.source_id AS source_id,
        a.unique_key AS unique_key,
        splitByChar('/', COALESCE(a.repo_full_name, ''))[1] AS project_key,
        splitByChar('/', COALESCE(a.repo_full_name, ''))[2] AS repo_slug,
        COALESCE(a.pr_id, 0) AS pr_id,
        COALESCE(a.pr_id, 0) AS pr_number,
        'review' AS event_kind,
        if(a.kind = 'approval', 'approved', COALESCE(a.kind, '')) AS review_state,
        COALESCE(a.actor_nickname, '') AS actor_login,
        COALESCE(a.actor_display_name, '') AS actor_name,
        lower(trimBoth(COALESCE(a.actor_account_id, ''))) AS actor_account_id,
        parseDateTimeBestEffortOrNull(a.event_date) AS created_at,
        a._airbyte_extracted_at AS _airbyte_extracted_at
    FROM {{ source('bronze_bitbucket_cloud', 'pull_request_activity') }} AS a FINAL
    WHERE a.kind IN ('approval', 'changes_requested')

    UNION ALL

    SELECT
        c.tenant_id AS tenant_id,
        c.source_id AS source_id,
        c.unique_key AS unique_key,
        splitByChar('/', COALESCE(c.repo_full_name, ''))[1] AS project_key,
        splitByChar('/', COALESCE(c.repo_full_name, ''))[2] AS repo_slug,
        COALESCE(c.pr_id, 0) AS pr_id,
        COALESCE(c.pr_id, 0) AS pr_number,
        'comment' AS event_kind,
        '' AS review_state,
        COALESCE(c.author_nickname, '') AS actor_login,
        COALESCE(c.author_display_name, '') AS actor_name,
        lower(trimBoth(COALESCE(c.author_account_id, ''))) AS actor_account_id,
        parseDateTimeBestEffortOrNull(c.created_on) AS created_at,
        c._airbyte_extracted_at AS _airbyte_extracted_at
    FROM {{ source('bronze_bitbucket_cloud', 'pull_request_comments') }} AS c FINAL
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
    'insight_bitbucket_cloud' AS data_source,
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
