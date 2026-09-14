-- depends_on: {{ ref('gitlab__bronze_promoted') }}
{{ config(
    materialized='table',
    schema='staging',
    tags=['gitlab']
) }}

-- Commit author e-mails that no GitLab account claims, with the name the
-- commits carry.
--
-- `gitlab__account_emails` collects the e-mails GitLab could match to an
-- account. What is left over is an address the vendor knows nothing about,
-- and gold attributes commits BY e-mail, so those commits reach no person at
-- all. Nothing automatic can resolve them; they are published as accounts of
-- their own so an operator can see them and say whose they are
-- (gitlab__identity_inputs).
--
-- Every unclaimed address is emitted, CI and service identities included.
-- What an address means is the operator's decision.

WITH walked_commits AS (
    SELECT
        lower(trimBoth(COALESCE(author_email, ''))) AS email,
        COALESCE(author_name, '') AS author_name,
        tenant_id,
        source_id,
        max(parseDateTimeBestEffortOrNull(authored_date)) AS seen_at
    FROM {{ source('bronze_gitlab', 'commits') }} FINAL
    WHERE COALESCE(author_email, '') != ''
    GROUP BY email, author_name, tenant_id, source_id
),

-- A squash-merged or fork-sourced request's commits reach no ref the proxy
-- clones, so their authors reach no other stream.
request_commits AS (
    SELECT
        lower(trimBoth(COALESCE(author_email, ''))) AS email,
        COALESCE(author_name, '') AS author_name,
        tenant_id,
        source_id,
        max(parseDateTimeBestEffortOrNull(authored_date)) AS seen_at
    FROM {{ source('bronze_gitlab', 'pull_request_commits') }} FINAL
    WHERE COALESCE(author_email, '') != ''
    GROUP BY email, author_name, tenant_id, source_id
),

every_author AS (
    SELECT * FROM walked_commits
    UNION ALL
    SELECT * FROM request_commits
),

candidates AS (
    SELECT *
    FROM every_author
    WHERE email != ''
      AND seen_at IS NOT NULL
)

SELECT
    c.email AS email,
    -- The name from their most recent commit: one person spells it several
    -- ways, and the freshest spelling is the one an operator will recognise.
    assumeNotNull(argMax(c.author_name, c.seen_at)) AS author_name,
    c.tenant_id AS tenant_id,
    c.source_id AS source_id,
    assumeNotNull(max(c.seen_at)) AS last_committed_at,
    sum(1) AS commit_idents
FROM candidates AS c
LEFT ANTI JOIN {{ ref('gitlab__account_emails') }} AS owned
    ON owned.email = c.email
    AND owned.tenant_id = c.tenant_id
    AND owned.source_id = c.source_id
GROUP BY c.email, c.tenant_id, c.source_id
