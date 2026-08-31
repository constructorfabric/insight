-- Build-integrity check (untagged → error severity under `dbt build`).
-- The runtime joins `identity.person_map` on exact string equality, so an
-- unnormalized key matches no map row and the person silently misses that
-- activity. The map's side is asserted by assert_person_map_email_normalized.
-- An EMPTY entity_id is legal only on a row that carries an account key —
-- such a row resolves through `identity.account_assignment` instead; with
-- neither key the row can never resolve and must not have survived the build.
SELECT
    entity_id,
    measure_key,
    count() AS row_count
FROM {{ ref('git_metric_observations') }}
WHERE entity_id != lower(trimBoth(entity_id))
   OR (entity_id = '' AND coalesce(account_id, '') = '')
GROUP BY entity_id, measure_key
