-- depends_on: {{ ref('gitlab__bronze_promoted') }}
{{ config(
    materialized='incremental',
    unique_key='unique_key',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    schema='staging',
    tags=['gitlab', 'silver:class_git_pr_review_events']
) }}

-- Review verdicts and comments as one event stream. Both come from the notes:
-- a verdict is a system note GitLab wrote when someone approved, withdrew an
-- approval or requested changes; a comment is a note a person wrote.
WITH projects AS (
    SELECT
        tenant_id,
        source_id,
        id AS project_id,
        COALESCE(namespace_full_path, '') AS project_key,
        COALESCE(path, '') AS repo_slug
    FROM {{ source('bronze_gitlab', 'repositories') }} FINAL
),

-- One address per account for the class's flat actor_email: a real address
-- outranks the noreply form, and the earliest-observed pair wins so the pick
-- is stable across syncs.
account_email AS (
    SELECT
        tenant_id,
        source_id,
        account_id,
        argMin(email, (email LIKE '%@users.noreply.%', observed_at, email)) AS email
    FROM {{ ref('gitlab__account_emails') }}
    GROUP BY tenant_id, source_id, account_id
),

events AS (
    SELECT
        tenant_id,
        source_id,
        unique_key,
        project_id,
        COALESCE(mr_iid, 0) AS pr_id,
        if(COALESCE(system, false), 'review', 'comment') AS event_kind,
        multiIf(
            NOT COALESCE(system, false), '',
            body = 'approved this merge request', 'approved',
            body = 'unapproved this merge request', 'dismissed',
            'changes_requested'
        ) AS review_state,
        COALESCE(author_username, '') AS actor_login,
        COALESCE(author_name, '') AS actor_name,
        toString(COALESCE(author_id, 0)) AS actor_account_id,
        parseDateTimeBestEffortOrNull(created_at) AS created_at,
        _airbyte_extracted_at
    FROM {{ source('bronze_gitlab', 'pull_request_notes') }} FINAL
    WHERE NOT COALESCE(system, false)
       OR body IN ('approved this merge request', 'unapproved this merge request')
       OR body LIKE 'requested changes%'
)

SELECT
    e.tenant_id AS tenant_id,
    e.source_id AS source_id,
    e.unique_key AS unique_key,
    p.project_key AS project_key,
    p.repo_slug AS repo_slug,
    e.pr_id AS pr_id,
    e.pr_id AS pr_number,
    e.event_kind AS event_kind,
    e.review_state AS review_state,
    e.actor_login AS actor_login,
    e.actor_name AS actor_name,
    e.actor_account_id AS actor_account_id,
    COALESCE(ae.email, '') AS actor_email,
    e.created_at AS created_at,
    'insight_gitlab' AS data_source,
    toUnixTimestamp64Milli(now64()) AS _version,
    e._airbyte_extracted_at AS _airbyte_extracted_at
FROM events AS e
INNER JOIN projects AS p
    ON p.tenant_id = e.tenant_id
    AND p.source_id = e.source_id
    AND p.project_id = e.project_id
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
