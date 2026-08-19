-- INVARIANT: `id DESC` breaks ties within a `created_at`. The journal's slot
-- allocator can stamp two observations of one account identically, and the
-- larger id is the later operator decision.
-- SAFETY: `LIMIT 1 BY` picks the winning decision, not a duplicate row version.
-- `identity_persons` is a plain MergeTree replaced by an atomic swap, so it
-- carries no versions to dedup.

{{ config(
    materialized='view',
    schema='identity',
    tags=['identity', 'identity:map']
) }}

SELECT
    insight_source_type                  AS source_type,
    insight_source_id                    AS source_id,
    trimBoth(value_effective)            AS account_id,
    person_id,
    created_at
FROM identity.identity_persons
WHERE value_type = 'id'
  AND value_effective IS NOT NULL
  AND trimBoth(value_effective) != ''
ORDER BY
    source_type,
    source_id,
    account_id,
    created_at DESC,
    id DESC
LIMIT 1 BY source_type, source_id, account_id
