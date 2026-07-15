-- Build-integrity check (untagged → error severity under `dbt build`).
-- Unified entity ids for persons are lowercased emails; the runtime and the
-- cohort view join on exact string equality, so an empty, mixed-case, or
-- non-email id (a source's user-id fallback leaking through the gate)
-- silently drops the person from every surface.
SELECT
    entity_id,
    measure_key,
    count() AS row_count
FROM {{ ref('collab_metric_observations') }}
WHERE entity_id = ''
   OR entity_id != lower(entity_id)
   OR entity_id NOT LIKE '%@%'
GROUP BY entity_id, measure_key
