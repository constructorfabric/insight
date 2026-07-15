-- Build-integrity check (untagged → error severity under `dbt build`).
-- Every collaboration measure is day-grain: exactly one row per
-- (tenant, entity, date, measure, dimensions, subject). A duplicate means
-- FINAL dedup regressed on a class read or a measure branch fanned out —
-- sums and distinct counts would silently inflate.
SELECT
    tenant_id,
    entity_id,
    metric_date,
    measure_key,
    dimensions,
    subject_key,
    count() AS row_count
FROM {{ ref('collab_metric_observations') }}
GROUP BY tenant_id, entity_id, metric_date, measure_key, dimensions, subject_key
HAVING count() > 1
