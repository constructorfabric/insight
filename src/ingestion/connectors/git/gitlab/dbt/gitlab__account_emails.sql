-- depends_on: {{ ref('gitlab__bronze_promoted') }}
{{ config(
    materialized='table',
    schema='staging',
    tags=['gitlab']
) }}

-- Every (GitLab account, e-mail) pair the connector has seen, one row per pair.
--
-- Commits carry an e-mail and no account; merge requests carry an account and
-- no e-mail. Neither side alone lets a commit or a request reach a person, so
-- this model collects the pairs that name both:
--   1. the commit author lookup — the proxy enumerates each project's authors,
--      and /users?search= names the account whose address matches
--   2. the instance directory (/users), where the token may read it: `email`
--      for an administrator, `public_email` and `commit_email` for anyone
--   3. the group and project rosters, which carry an address on the editions
--      and tokens that expose one
--   4. the private commit address `{id}-{username}@<commit-email host>`, which
--      states the numeric account id itself; the connector stamps that id only
--      when the host is this instance's own
--
-- Several e-mails per account is normal and every one of them is kept: a
-- person who changes address still owns what they committed under the old one.

WITH resolved_authors AS (
    SELECT
        toString(author_account_id) AS account_id,
        lower(trimBoth(COALESCE(author_email, ''))) AS email,
        tenant_id,
        source_id,
        'bronze_gitlab.commit_authors.author_email' AS observed_in,
        max(parseDateTimeBestEffortOrNull(collected_at)) AS seen_at
    FROM {{ source('bronze_gitlab', 'commit_authors') }} FINAL
    WHERE COALESCE(author_account_id, 0) > 0
      AND COALESCE(author_email, '') != ''
    GROUP BY account_id, email, tenant_id, source_id, observed_in
),

-- WORKAROUND: ClickHouse substitutes the alias `email` into the array literal
-- that reads the `email` column, so the unnest runs one subquery below it.
directory_addresses AS (
    SELECT
        id,
        tenant_id,
        source_id,
        collected_at,
        address,
        field
    FROM {{ source('bronze_gitlab', 'users') }} FINAL
    ARRAY JOIN
        [email, public_email, commit_email] AS address,
        ['email', 'public_email', 'commit_email'] AS field
    WHERE COALESCE(id, 0) > 0
      AND COALESCE(address, '') != ''
),

directory_emails AS (
    SELECT
        toString(id) AS account_id,
        lower(trimBoth(address)) AS email,
        tenant_id,
        source_id,
        concat('bronze_gitlab.users.', field) AS observed_in,
        max(parseDateTimeBestEffortOrNull(collected_at)) AS seen_at
    FROM directory_addresses
    GROUP BY account_id, email, tenant_id, source_id, observed_in
),

roster_addresses AS (
    SELECT
        id,
        tenant_id,
        source_id,
        collected_at,
        address,
        field
    FROM {{ source('bronze_gitlab', 'group_members') }} FINAL
    ARRAY JOIN
        [email, public_email] AS address,
        ['email', 'public_email'] AS field
    WHERE COALESCE(id, 0) > 0
      AND COALESCE(address, '') != ''
),

roster_emails AS (
    SELECT
        toString(id) AS account_id,
        lower(trimBoth(address)) AS email,
        tenant_id,
        source_id,
        concat('bronze_gitlab.group_members.', field) AS observed_in,
        max(parseDateTimeBestEffortOrNull(collected_at)) AS seen_at
    FROM roster_addresses
    GROUP BY account_id, email, tenant_id, source_id, observed_in
),

-- The private commit address states the account id it was issued to. The
-- connector stamps author_account_id from it only for this instance's own
-- commit-email host, so an address minted elsewhere claims nothing here.
noreply_commits AS (
    SELECT
        toString(author_account_id) AS account_id,
        lower(trimBoth(COALESCE(author_email, ''))) AS email,
        tenant_id,
        source_id,
        'bronze_gitlab.commits.author_email' AS observed_in,
        max(parseDateTimeBestEffortOrNull(authored_date)) AS seen_at
    FROM {{ source('bronze_gitlab', 'commits') }} FINAL
    WHERE COALESCE(author_account_id, 0) > 0
    GROUP BY account_id, email, tenant_id, source_id, observed_in
),

noreply_request_commits AS (
    SELECT
        toString(author_account_id) AS account_id,
        lower(trimBoth(COALESCE(author_email, ''))) AS email,
        tenant_id,
        source_id,
        'bronze_gitlab.pull_request_commits.author_email' AS observed_in,
        max(parseDateTimeBestEffortOrNull(authored_date)) AS seen_at
    FROM {{ source('bronze_gitlab', 'pull_request_commits') }} FINAL
    WHERE COALESCE(author_account_id, 0) > 0
    GROUP BY account_id, email, tenant_id, source_id, observed_in
),

observations AS (
    SELECT * FROM resolved_authors
    UNION ALL
    SELECT * FROM directory_emails
    UNION ALL
    SELECT * FROM roster_emails
    UNION ALL
    SELECT * FROM noreply_commits
    UNION ALL
    SELECT * FROM noreply_request_commits
),

-- The noreply pattern yields '' for an address that states no account id, and
-- a row whose date will not parse carries no usable observation.
every_pair AS (
    SELECT *
    FROM observations
    WHERE account_id != ''
      AND email != ''
      AND seen_at IS NOT NULL
)

SELECT
    account_id,
    email,
    tenant_id,
    source_id,
    -- Which source named the pair first, for provenance on the claim.
    -- assumeNotNull because argMin and min inherit the ordering column's
    -- nullability, and these feed silver.identity_inputs where a Nullable
    -- would widen a column every connector shares. Every row reaching here
    -- passed the IS NOT NULL guard above.
    assumeNotNull(argMin(observed_in, (seen_at, observed_in))) AS observed_in,
    assumeNotNull(min(seen_at)) AS observed_at
FROM every_pair
-- The full connection scope, not just the pair: the same numeric id names
-- different accounts on different GitLab instances.
GROUP BY account_id, email, tenant_id, source_id
