-- depends_on: {{ ref('github__bronze_promoted') }}
{{ config(
    materialized='table',
    schema='staging',
    tags=['github']
) }}

-- Every (GitHub account, e-mail) pair the connector has seen, one row per pair.
--
-- Commits carry an e-mail and no account; the roster carries an account and,
-- only where a member publishes one, an e-mail. Neither side alone lets a
-- commit reach a person, so this model collects the pairs that name both.
--
-- Four independent sources, weakest last:
--   1. the commit author lookup, which resolves an e-mail GitHub could match
--      to an account whether or not the person ever opened a pull request
--   2. the commit's own account on a pull-request commit, as GitHub matched it
--   3. the profile e-mail on the pull-request author
--   4. the noreply address, whose modern form encodes the numeric account id
--      it was issued to and so names its own account without any lookup (the
--      pre-2017 form carries only the login and names no account here)
--
-- Several e-mails per account is normal and every one of them is kept: a person
-- who changes address still owns what they committed under the old one.

WITH resolved_authors AS (
    SELECT
        toString(author_id) AS account_id,
        lower(trimBoth(COALESCE(author_email, ''))) AS email,
        tenant_id,
        source_id,
        'bronze_github.commit_authors.author_email' AS observed_in,
        max(parseDateTimeBestEffortOrNull(collected_at)) AS seen_at
    FROM {{ source('bronze_github', 'commit_authors') }} FINAL
    -- 0 is the connector's stand-in for "GitHub matched no account to this
    -- e-mail" — it does NOT exclude bots, which carry real numeric ids.
    WHERE COALESCE(author_id, 0) > 0
      AND COALESCE(author_email, '') != ''
    GROUP BY account_id, email, tenant_id, source_id, observed_in
),

commit_accounts AS (
    SELECT
        toString(author_id) AS account_id,
        lower(trimBoth(COALESCE(author_email, ''))) AS email,
        tenant_id,
        source_id,
        'bronze_github.pull_request_commits.author_email' AS observed_in,
        max(parseDateTimeBestEffortOrNull(authored_date)) AS seen_at
    FROM {{ source('bronze_github', 'pull_request_commits') }} FINAL
    WHERE COALESCE(author_id, 0) > 0
      AND COALESCE(author_email, '') != ''
    GROUP BY account_id, email, tenant_id, source_id, observed_in
),

committer_accounts AS (
    SELECT
        toString(committer_id) AS account_id,
        lower(trimBoth(COALESCE(committer_email, ''))) AS email,
        tenant_id,
        source_id,
        'bronze_github.pull_request_commits.committer_email' AS observed_in,
        max(parseDateTimeBestEffortOrNull(committed_date)) AS seen_at
    FROM {{ source('bronze_github', 'pull_request_commits') }} FINAL
    WHERE COALESCE(committer_id, 0) > 0
      AND COALESCE(committer_email, '') != ''
    GROUP BY account_id, email, tenant_id, source_id, observed_in
),

profile_emails AS (
    SELECT
        toString(author_id) AS account_id,
        lower(trimBoth(COALESCE(author_email, ''))) AS email,
        tenant_id,
        source_id,
        'bronze_github.pull_request_diff_stats.author_email' AS observed_in,
        max(parseDateTimeBestEffortOrNull(updated_at)) AS seen_at
    FROM {{ source('bronze_github', 'pull_request_diff_stats') }} FINAL
    WHERE COALESCE(author_id, 0) > 0
      AND COALESCE(author_email, '') != ''
    GROUP BY account_id, email, tenant_id, source_id, observed_in
),

-- `12345678+octocat@users.noreply.github.com` states the numeric account id
-- it was issued to. The pre-2017 form (`octocat@users.noreply.github.com`)
-- carries only the login, which is not an account key: those addresses yield
-- '' here and fall through to github__unowned_commit_emails, becoming
-- commit-email accounts of their own — auto-minted as separate persons where
-- nothing else claims the address — for an operator to merge.
noreply_commits AS (
    SELECT
        extract(COALESCE(author_email, ''), '^([0-9]+)\\+[^@]+@users\\.noreply\\.github\\.com$') AS account_id,
        lower(trimBoth(COALESCE(author_email, ''))) AS email,
        tenant_id,
        source_id,
        'bronze_github.commits.author_email' AS observed_in,
        max(parseDateTimeBestEffortOrNull(authored_date)) AS seen_at
    FROM {{ source('bronze_github', 'commits') }} FINAL
    WHERE author_email LIKE '%@users.noreply.github.com'
    GROUP BY account_id, email, tenant_id, source_id, observed_in
),

observations AS (
    SELECT * FROM resolved_authors
    UNION ALL
    SELECT * FROM commit_accounts
    UNION ALL
    SELECT * FROM committer_accounts
    UNION ALL
    SELECT * FROM profile_emails
    UNION ALL
    SELECT * FROM noreply_commits
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
    -- nullability: these two feed silver.identity_inputs.value_field_name and
    -- its `_version`, where a Nullable would widen a column every connector
    -- shares and ReplacingMergeTree rejects a nullable version outright. Every
    -- row reaching here passed the IS NOT NULL guard above.
    assumeNotNull(argMin(observed_in, seen_at)) AS observed_in,
    assumeNotNull(min(seen_at)) AS observed_at
FROM every_pair
-- The full connection scope, not just the pair: several GitHub connections
-- share this bronze namespace, and the same numeric id names different
-- accounts on different hosts (github.com and an Enterprise Server both count
-- from 1) — a pair collapsed across scopes lands its claim under an arbitrary
-- one.
GROUP BY account_id, email, tenant_id, source_id
