-- depends_on: {{ ref('bitbucket_cloud__bronze_promoted') }}
{{ config(
    materialized='table',
    schema='staging',
    tags=['bitbucket-cloud']
) }}

-- Commit author e-mails that no Bitbucket account claims, with the name the
-- commits carry.
--
-- `bitbucket_cloud__account_emails` collects the e-mails Bitbucket could match
-- to an account. What is left over is an address the vendor knows nothing about
-- — commonly a work address its owner never added to their Atlassian account —
-- and gold attributes commits BY e-mail, so those commits reach no person.
--
-- Nothing automatic can resolve them: the account they belong to exists only in
-- a human's head. They are published as accounts of their own so an operator
-- can see them and say whose they are (bitbucket_cloud__identity_inputs), which
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
    FROM {{ source('bronze_bitbucket_cloud', 'commits') }} FINAL
    WHERE COALESCE(author_email, '') != ''
    GROUP BY email, author_name, tenant_id, source_id
),

-- The head commits of an unmerged or declined pull request are reachable from
-- no branch the proxy clones, and a declined one never becomes reachable, so
-- their authors appear in no other stream.
pull_request_commits AS (
    SELECT
        lower(trimBoth(COALESCE(author_email, ''))) AS email,
        COALESCE(author_name, '') AS author_name,
        tenant_id,
        source_id,
        max(parseDateTimeBestEffortOrNull(committed_date)) AS seen_at
    FROM {{ source('bronze_bitbucket_cloud', 'pull_request_commits') }} FINAL
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
    c.tenant_id AS tenant_id,
    c.source_id AS source_id,
    assumeNotNull(max(c.seen_at)) AS last_committed_at,
    sum(1) AS commit_idents
FROM candidates AS c
-- Scoped to the connection, not the address: several Bitbucket connections
-- share this bronze namespace, and an account claiming an e-mail in one says
-- nothing about the same e-mail in another.
LEFT ANTI JOIN {{ ref('bitbucket_cloud__account_emails') }} AS owned
    ON  owned.email     = c.email
    AND owned.tenant_id = c.tenant_id
    AND owned.source_id = c.source_id
GROUP BY c.email, c.tenant_id, c.source_id
