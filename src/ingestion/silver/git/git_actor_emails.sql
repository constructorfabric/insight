{{ config(
    materialized='view',
    schema='identity',
    alias='git_actor_emails',
    tags=['gold']
) }}

-- Identity bridge: (tenant_id, data_source, actor_name) -> resolved work
-- email, for git rows that carry an actor name but no email (pull request
-- authors on sources whose APIs hide emails).
--
-- A view, not a table: dominant-email election must be recomputed over the
-- full commit history on every read, so new commits retroactively re-elect
-- the mapping — the same retroactivity argument that keeps file
-- classification out of incremental silver.
--
-- Tagged 'gold' although conceptually silver-layer: the deploy gate builds
-- `dbt run --select tag:gold`, and this view must exist before
-- git_metric_observations compiles. dbt orders it first via ref().
--
-- Tiers, never guessing:
--   (a) commit co-occurrence — commits carry both author_name and
--       author_email; the dominant email per (tenant, name) wins, a tie
--       between distinct emails at the top count resolves to no row.
--   (b) directory identity_inputs — display_name -> email from connector
--       identity sources; ambiguous display names resolve to no row.
-- Rows exist only for resolved names, so consumers LEFT JOIN and drop
-- misses (honest exclusion). `data_source` is carried so a future
-- per-source refinement (connector-owned username inputs) is non-breaking;
-- today's resolution is source-agnostic.
--
-- The sipHash-to-UUID tenant join mirrors the documented convention in
-- identity_inputs_from_history.sql (identity_inputs keys tenants by hashed
-- UUID; git class rows carry the raw tenant string).

WITH
git_tenants AS (
    SELECT DISTINCT tenant_id, data_source
    FROM {{ ref('class_git_commits') }} FINAL
    UNION DISTINCT
    SELECT DISTINCT tenant_id, data_source
    FROM {{ ref('class_git_pull_requests') }} FINAL
),
commit_pairs AS (
    SELECT
        tenant_id,
        lower(trimBoth(author_name)) AS actor_name,
        lower(author_email) AS email,
        uniqExact(commit_hash) AS commit_count
    FROM {{ ref('class_git_commits') }} FINAL
    WHERE author_name != ''
      AND author_email != ''
    GROUP BY tenant_id, actor_name, email
),
commit_dominant AS (
    SELECT
        tenant_id,
        actor_name,
        if(uniqExact(email) = 1, any(email), CAST(NULL AS Nullable(String))) AS email
    FROM (
        SELECT
            tenant_id,
            actor_name,
            email,
            commit_count,
            max(commit_count) OVER (PARTITION BY tenant_id, actor_name) AS max_count
        FROM commit_pairs
    )
    WHERE commit_count = max_count
    GROUP BY tenant_id, actor_name
),
latest_identity_values AS (
    SELECT
        insight_tenant_id,
        insight_source_type,
        source_account_id,
        value_type,
        argMax(value, _version) AS value,
        argMax(operation_type, _version) AS last_operation
    FROM {{ ref('identity_inputs') }} FINAL
    WHERE value_type IN ('display_name', 'email')
    GROUP BY insight_tenant_id, insight_source_type, source_account_id, value_type
),
directory_pairs AS (
    SELECT
        names.insight_tenant_id AS insight_tenant_id,
        lower(trimBoth(names.value)) AS actor_name,
        lower(emails.value) AS email
    FROM latest_identity_values AS names
    INNER JOIN latest_identity_values AS emails
        ON emails.insight_tenant_id = names.insight_tenant_id
        AND emails.insight_source_type = names.insight_source_type
        AND emails.source_account_id = names.source_account_id
        AND emails.value_type = 'email'
    WHERE names.value_type = 'display_name'
      AND names.last_operation = 'UPSERT'
      AND emails.last_operation = 'UPSERT'
      AND names.value != ''
      AND emails.value != ''
),
directory_dominant AS (
    SELECT
        insight_tenant_id,
        actor_name,
        if(uniqExact(email) = 1, any(email), CAST(NULL AS Nullable(String))) AS email
    FROM directory_pairs
    GROUP BY insight_tenant_id, actor_name
),
tenant_hashes AS (
    SELECT DISTINCT
        tenant_id,
        toUUID(UUIDNumToString(sipHash128(coalesce(tenant_id, '')))) AS insight_tenant_id
    FROM git_tenants
),
actor_names AS (
    SELECT tenant_id, actor_name FROM commit_dominant
    UNION DISTINCT
    SELECT
        tenant_hashes.tenant_id AS tenant_id,
        directory_dominant.actor_name AS actor_name
    FROM directory_dominant
    INNER JOIN tenant_hashes
        ON tenant_hashes.insight_tenant_id = directory_dominant.insight_tenant_id
)
SELECT
    git_tenants.tenant_id AS tenant_id,
    git_tenants.data_source AS data_source,
    actor_names.actor_name AS actor_name,
    assumeNotNull(coalesce(commit_dominant.email, directory_dominant.email)) AS email
FROM actor_names
INNER JOIN git_tenants
    ON git_tenants.tenant_id = actor_names.tenant_id
LEFT JOIN commit_dominant
    ON commit_dominant.tenant_id = actor_names.tenant_id
    AND commit_dominant.actor_name = actor_names.actor_name
LEFT JOIN tenant_hashes
    ON tenant_hashes.tenant_id = actor_names.tenant_id
LEFT JOIN directory_dominant
    ON directory_dominant.insight_tenant_id = tenant_hashes.insight_tenant_id
    AND directory_dominant.actor_name = actor_names.actor_name
WHERE coalesce(commit_dominant.email, directory_dominant.email) IS NOT NULL
  AND coalesce(commit_dominant.email, directory_dominant.email) != ''
