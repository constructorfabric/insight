-- Build-integrity check (untagged → error severity under `dbt build`).
-- Unified entity ids for persons are canonical person UUIDs since the
-- identity cutover; the runtime and the cohort relation join on exact string
-- equality, so an empty, mixed-case, non-UUID id — a source email leaking
-- past the resolve gate — or the nil UUID silently drops the person from
-- every surface.
SELECT
    entity_id,
    measure_key,
    count() AS row_count
FROM {{ ref('git_metric_observations') }}
WHERE NOT match(entity_id, '^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$')
   OR entity_id = '00000000-0000-0000-0000-000000000000'
GROUP BY entity_id, measure_key
