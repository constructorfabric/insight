-- depends_on: {{ ref('github__bronze_promoted') }}
{{ config(
    materialized='table',
    schema='staging',
    tags=['github']
) }}

-- Commit author e-mails that no GitHub account claims, with the name the
-- commits carry.
--
-- `github__account_emails` collects the e-mails GitHub could match to an
-- account. What is left over is an address the vendor knows nothing about —
-- almost always a work address its owner never verified on GitHub — and gold
-- attributes commits BY e-mail, so those commits reach no person at all.
--
-- Nothing automatic can resolve them: the account they belong to exists only
-- in a human's head. They are published as accounts of their own so an
-- operator can see them and say whose they are (github__identity_inputs), which
-- is the only mechanism that can close the gap.
--
-- Every unclaimed address is emitted, CI and service identities included. What
-- an address means is the operator's decision — `exclude` is one of their verbs
-- — and a filter here would hide the decision instead of making it.

WITH walked_commits AS (
    SELECT
        lower(trimBoth(COALESCE(author_email, ''))) AS email,
        COALESCE(author_name, '') AS author_name,
        tenant_id,
        source_id,
        max(parseDateTimeBestEffortOrNull(authored_date)) AS seen_at
    FROM {{ source('bronze_github', 'commits') }} FINAL
    WHERE COALESCE(author_email, '') != ''
    GROUP BY email, author_name, tenant_id, source_id
),

-- A fork's head commits live under refs/pull/*, which a clone never fetches, so
-- their authors reach no other stream.
pull_request_commits AS (
    SELECT
        lower(trimBoth(COALESCE(author_email, ''))) AS email,
        COALESCE(author_name, '') AS author_name,
        tenant_id,
        source_id,
        max(parseDateTimeBestEffortOrNull(authored_date)) AS seen_at
    FROM {{ source('bronze_github', 'pull_request_commits') }} FINAL
    WHERE COALESCE(author_email, '') != ''
    GROUP BY email, author_name, tenant_id, source_id
),

every_author AS (
    SELECT * FROM walked_commits
    UNION ALL
    SELECT * FROM pull_request_commits
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
    any(c.tenant_id) AS tenant_id,
    any(c.source_id) AS source_id,
    assumeNotNull(max(c.seen_at)) AS last_committed_at,
    sum(1) AS commit_idents
FROM candidates AS c
LEFT ANTI JOIN {{ ref('github__account_emails') }} AS owned
    ON owned.email = c.email
GROUP BY c.email
