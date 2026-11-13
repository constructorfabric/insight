-- Build-integrity check (untagged → error severity under `dbt build`).
-- The runtime joins `identity.person_map` on exact string equality, so an
-- unnormalized key matches no map row and the person silently misses that
-- activity. The map's side is asserted by assert_person_map_email_normalized.
-- An EMPTY entity_id with an account key resolves through
-- `identity.account_assignment` instead. With neither key the row is an
-- unassigned issue's and counts only as unattributed, so it must still name
-- the tracker source it came from.
SELECT
    entity_id,
    measure_key,
    count() AS row_count
FROM {{ ref('task_metric_observations') }}
WHERE entity_id != lower(trimBoth(entity_id))
   OR (entity_id = '' AND coalesce(account_id, '') = '' AND coalesce(account_source_id, '') = '')
GROUP BY entity_id, measure_key
