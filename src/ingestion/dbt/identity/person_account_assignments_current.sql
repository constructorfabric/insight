{{ config(
    materialized='view',
    schema='identity',
    tags=['identity']
) }}

-- Current account→person assignment: for every stable source account, the
-- person it is bound to right now.
--
-- A VIEW, not a published table: `identity.identity_persons` is replaced
-- wholesale by an atomic EXCHANGE, so a view over it is always consistent
-- with the latest published journal and carries no staleness window of its
-- own. Materializing would add a second publication step that could only
-- ever be older than its source.
--
-- Latest-wins is keyed by ACCOUNT, not by person: partitioning by person
-- answers "which account does this person use", and the two disagree
-- precisely when an account is rebound from one person to another — the
-- case this relation exists to resolve. Mirrors the account-keyed window in
-- the service's own resolver.
--
-- No FINAL: identity_persons is a plain MergeTree holding a full snapshot,
-- so `LIMIT 1 BY` here is choosing the winning observation, not collapsing
-- duplicate parts.

SELECT
    insight_tenant_id,
    insight_source_type,
    insight_source_id,
    value_id      AS source_account_id,
    person_id,
    created_at    AS assigned_at,
    _synced_at
FROM identity.identity_persons
WHERE value_type = 'id'
  AND value_id IS NOT NULL
  AND value_id != ''
ORDER BY
    insight_tenant_id,
    insight_source_type,
    insight_source_id,
    value_id,
    created_at DESC,
    id DESC
LIMIT 1 BY
    insight_tenant_id,
    insight_source_type,
    insight_source_id,
    value_id
