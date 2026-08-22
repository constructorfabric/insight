-- depends_on: {{ ref('bitbucket_cloud__bronze_promoted') }}
{{ config(
    materialized='table',
    schema='staging',
    tags=['bitbucket-cloud']
) }}

-- Every (Bitbucket account, e-mail) pair the connector has seen, one row per
-- pair.
--
-- Commits carry an e-mail and no account; the workspace roster carries an
-- account and no e-mail. Neither side alone lets a commit reach a person, so
-- this model collects the pairs that name both.
--
-- Two sources, weakest last:
--   1. the commit author lookup, which resolves an e-mail Bitbucket could
--      match to an account whether or not the person ever opened a pull
--      request
--   2. the account on a pull-request commit, which Bitbucket names inline on
--      a response the connector already fetches
--
-- Bitbucket has no noreply-address form, so unlike GitHub there is no third,
-- lookup-free source: an account is named by the vendor or not at all.
--
-- Several e-mails per account is normal and every one of them is kept: a
-- person who changes address still owns what they committed under the old one.

WITH resolved_authors AS (
    SELECT
        lower(trimBoth(COALESCE(author_account_id, ''))) AS account_id,
        lower(trimBoth(COALESCE(author_email, ''))) AS email,
        tenant_id,
        source_id,
        'bronze_bitbucket_cloud.commit_authors.author_email' AS observed_in,
        max(parseDateTimeBestEffortOrNull(collected_at)) AS seen_at
    FROM {{ source('bronze_bitbucket_cloud', 'commit_authors') }} FINAL
    WHERE COALESCE(author_account_id, '') != ''
      AND COALESCE(author_email, '') != ''
    GROUP BY account_id, email, tenant_id, source_id, observed_in
),

pull_request_commit_accounts AS (
    SELECT
        lower(trimBoth(COALESCE(author_account_id, ''))) AS account_id,
        lower(trimBoth(COALESCE(author_email, ''))) AS email,
        tenant_id,
        source_id,
        'bronze_bitbucket_cloud.pull_request_commits.author_email' AS observed_in,
        max(parseDateTimeBestEffortOrNull(committed_date)) AS seen_at
    FROM {{ source('bronze_bitbucket_cloud', 'pull_request_commits') }} FINAL
    WHERE COALESCE(author_account_id, '') != ''
      AND COALESCE(author_email, '') != ''
    GROUP BY account_id, email, tenant_id, source_id, observed_in
),

observations AS (
    SELECT * FROM resolved_authors
    UNION ALL
    SELECT * FROM pull_request_commit_accounts
),

-- A row whose date will not parse carries no usable observation.
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
    -- nullability: these two feed silver.identity_inputs.value_field_name and
    -- its `_version`, where a Nullable would widen a column every connector
    -- shares and ReplacingMergeTree rejects a nullable version outright. Every
    -- row reaching here passed the IS NOT NULL guard above.
    assumeNotNull(argMin(observed_in, seen_at)) AS observed_in,
    assumeNotNull(min(seen_at)) AS observed_at
FROM every_pair
-- The full connection scope, not just the pair: several Bitbucket connections
-- share this bronze namespace, and an account id is only unique within one.
GROUP BY account_id, email, tenant_id, source_id
