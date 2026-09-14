-- depends_on: {{ ref('gitlab__bronze_promoted') }}
{{ config(
    materialized='table',
    schema='staging',
    tags=['gitlab']
) }}

-- The display name GitLab shows for each account, from whichever stream
-- carried it. GitLab hands an address only to an administrator's token, so
-- for many accounts the name is all an operator reviewing an unbound account
-- has to go on.

WITH names AS (
    SELECT
        toString(id) AS account_id,
        COALESCE(name, '') AS name,
        tenant_id,
        source_id,
        'bronze_gitlab.users.name' AS observed_in,
        max(parseDateTimeBestEffortOrNull(collected_at)) AS seen_at
    FROM {{ source('bronze_gitlab', 'users') }} FINAL
    WHERE COALESCE(id, 0) > 0
      AND COALESCE(name, '') != ''
    GROUP BY account_id, name, tenant_id, source_id, observed_in

    UNION ALL

    SELECT
        toString(id) AS account_id,
        COALESCE(name, '') AS name,
        tenant_id,
        source_id,
        'bronze_gitlab.group_members.name' AS observed_in,
        max(parseDateTimeBestEffortOrNull(collected_at)) AS seen_at
    FROM {{ source('bronze_gitlab', 'group_members') }} FINAL
    WHERE COALESCE(id, 0) > 0
      AND COALESCE(name, '') != ''
    GROUP BY account_id, name, tenant_id, source_id, observed_in

    UNION ALL

    SELECT
        toString(author_account_id) AS account_id,
        COALESCE(author_name, '') AS name,
        tenant_id,
        source_id,
        'bronze_gitlab.commit_authors.author_name' AS observed_in,
        max(parseDateTimeBestEffortOrNull(collected_at)) AS seen_at
    FROM {{ source('bronze_gitlab', 'commit_authors') }} FINAL
    WHERE COALESCE(author_account_id, 0) > 0
      AND COALESCE(author_name, '') != ''
    GROUP BY account_id, name, tenant_id, source_id, observed_in

    UNION ALL

    SELECT
        toString(author_id) AS account_id,
        COALESCE(author_name, '') AS name,
        tenant_id,
        source_id,
        'bronze_gitlab.pull_requests.author_name' AS observed_in,
        max(parseDateTimeBestEffortOrNull(updated_at)) AS seen_at
    FROM {{ source('bronze_gitlab', 'pull_requests') }} FINAL
    WHERE COALESCE(author_id, 0) > 0
      AND COALESCE(author_name, '') != ''
    GROUP BY account_id, name, tenant_id, source_id, observed_in
)

-- One name per account: the most recently observed spelling.
SELECT
    account_id,
    tenant_id,
    source_id,
    assumeNotNull(argMax(name, seen_at)) AS name,
    assumeNotNull(argMax(observed_in, seen_at)) AS observed_in
FROM names
WHERE seen_at IS NOT NULL
GROUP BY account_id, tenant_id, source_id
