-- Build-integrity check (untagged → error severity under `dbt build`).
-- All four wiki measures are day-grain sums: exactly one row per (tenant,
-- entity, date, measure, dimensions, subject). A duplicate means FINAL dedup
-- regressed on a class read or the engagement join fanned out, silently
-- inflating the sums.
SELECT
    tenant_id,
    entity_id,
    metric_date,
    measure_key,
    dimensions,
    subject_key,
    count() AS row_count
FROM {{ ref('wiki_metric_observations') }}
GROUP BY tenant_id, entity_id, metric_date, measure_key, dimensions, subject_key
HAVING count() > 1
