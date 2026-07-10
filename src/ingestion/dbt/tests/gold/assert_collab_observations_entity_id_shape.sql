-- Build-integrity check (untagged → error severity under `dbt build`).
-- Unified entity ids for persons are lowercased emails (or lowercased user
-- ids where a source has no email); the runtime and the cohort view join on
-- exact string equality, so an empty or mixed-case id silently drops the
-- person from every surface.
SELECT
    entity_id,
    measure_key,
    count() AS row_count
FROM {{ ref('collab_metric_observations') }}
WHERE entity_id = ''
   OR entity_id != lower(entity_id)
GROUP BY entity_id, measure_key
