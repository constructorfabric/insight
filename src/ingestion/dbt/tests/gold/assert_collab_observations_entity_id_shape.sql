-- Build-integrity check (untagged → error severity under `dbt build`).
-- The runtime joins `identity.person_map` on exact string equality, so an
-- unnormalized or empty key matches no map row and the person silently misses
-- that activity. The map's side is asserted by assert_person_map_email_normalized.
SELECT
    entity_id,
    measure_key,
    count() AS row_count
FROM {{ ref('collab_metric_observations') }}
WHERE entity_id != lower(trimBoth(entity_id))
   OR entity_id = ''
GROUP BY entity_id, measure_key
